#![allow(static_mut_refs)]

use arrayvec::{ArrayString, ArrayVec};
use std::io::Write as _;
use std::{
    fmt::{Display, Write as _},
    fs::File,
    mem::MaybeUninit,
    panic::PanicHookInfo,
};

const MAGIC_NUMBER: u32 = 0xDEADB011;

#[derive(Debug)]
struct CrashReport {
    magic: u32,
    panic: bool,
    crash: bool,
    backtrace: ArrayVec<usize, 64>,
    message: ArrayString<512>,
    /// The last memory allocation that failed, with the heap's state at the time.
    alloc_failure: ArrayString<160>,
    /// Test builds: recent events with the heap's state (a ring, oldest at `next_event`).
    events: [ArrayString<80>; EVENTS],
    next_event: usize,
}

const EVENTS: usize = 12;

#[link_section = ".rtc_noinit"]
static mut REPORT: MaybeUninit<CrashReport> = MaybeUninit::uninit();

unsafe extern "C" {
    unsafe fn __real_esp_panic_handler(info: *mut core::ffi::c_void);
}

#[no_mangle]
pub unsafe extern "C" fn __wrap_esp_panic_handler(info: *mut core::ffi::c_void) {
    let report = REPORT.assume_init_mut();
    report.magic = MAGIC_NUMBER;
    report.crash = true;

    let mut pc: u32 = 0;
    let mut sp: u32 = 0;
    let mut next_pc: u32 = 0;

    esp_idf_svc::sys::esp_backtrace_get_start(&mut pc, &mut sp, &mut next_pc);
    let mut frame = esp_idf_svc::sys::esp_backtrace_frame_t {
        pc,
        sp,
        next_pc,
        exc_frame: core::ptr::null_mut(),
    };

    while !report.backtrace.is_full() {
        if !esp_idf_svc::sys::esp_backtrace_get_next_frame(&mut frame) {
            break;
        }
        if frame.pc == 0 {
            break;
        }
        // Xtensa stuff
        let pc = (frame.pc & 0x3FFF_FFFF) | 0x4000_0000;
        report.backtrace.push(pc as usize);
    }

    __real_esp_panic_handler(info);
}

fn get_location<'a>(info: &'a PanicHookInfo<'_>) -> &'a dyn Display {
    match info.location() {
        Some(location) => location,
        None => &"(unknown)",
    }
}

/// Record a failed memory allocation (the crash report shows the last one).
unsafe extern "C" fn alloc_failed(size: usize, caps: u32, _function: *const core::ffi::c_char) {
    use esp_idf_svc::sys::*;
    let report = REPORT.assume_init_mut();
    report.alloc_failure.clear();
    let _ = write!(
        report.alloc_failure,
        "Out of memory: {size} bytes (caps {caps:#x}); free {}, largest block {}, lowest free {}",
        heap_caps_get_free_size(MALLOC_CAP_8BIT),
        heap_caps_get_largest_free_block(MALLOC_CAP_8BIT),
        heap_caps_get_minimum_free_size(MALLOC_CAP_8BIT),
    );
}

/// Test builds: note an event and the heap's state in the crash report.
pub fn note(event: &dyn Display) {
    if !cfg!(diagnostics) {
        return;
    }
    use esp_idf_svc::sys::*;
    let report = unsafe { REPORT.assume_init_mut() };
    let slot = &mut report.events[report.next_event % EVENTS];
    slot.clear();
    let _ = unsafe {
        write!(
            slot,
            "{:7.1}s {event}: free {}, largest {}",
            esp_timer_get_time() as f64 / 1e6,
            heap_caps_get_free_size(MALLOC_CAP_8BIT),
            heap_caps_get_largest_free_block(MALLOC_CAP_8BIT),
        )
    };
    report.next_event = (report.next_event + 1) % EVENTS;
}

/// Setup crash and panic handlers.
pub fn setup() {
    // The report is kept across a crash's restart, and is otherwise uninitialized: start the
    // fields written before the SD card is read with nothing in them.
    let report = unsafe { REPORT.assume_init_mut() };
    if report.magic != MAGIC_NUMBER {
        report.alloc_failure.clear();
        report.events.iter_mut().for_each(|e| e.clear());
        report.next_event = 0;
    }
    unsafe {
        esp_idf_svc::sys::heap_caps_register_failed_alloc_callback(Some(alloc_failed));
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = unsafe { REPORT.assume_init_mut() };
        if report.magic == MAGIC_NUMBER {
            default_hook(info);
            return;
        }
        report.magic = MAGIC_NUMBER;
        report.panic = true;

        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or_default();
        let _ = writeln!(
            report.message,
            "PANIC: thread '{}' at {}:\n{}",
            thread_name,
            get_location(info),
            info.payload_as_str().unwrap_or_default(),
        );
        // Default hook will print some stuff, then abort(), which calls above panic handler.
        default_hook(info);
    }));
}

fn write_crash(crash: &CrashReport) -> std::io::Result<()> {
    let mut file = File::create("/sdcard/crash-log.txt")?;
    writeln!(file, "Crash Report")?;
    writeln!(file, "  FW: {}", env!("FIRMWARE_DISPLAY_VERSION"))?;
    writeln!(file, "  HW: {}", crate::hwinfo::get_hardware_version())?;
    writeln!(file, "  Serial: {}", crate::hwinfo::get_serial_number())?;
    writeln!(file)?;

    writeln!(file, "Message:\n{}\n", &crash.message)?;
    if !crash.alloc_failure.is_empty() {
        writeln!(file, "{}\n", &crash.alloc_failure)?;
    }
    let events = (0..EVENTS).map(|i| &crash.events[(crash.next_event + i) % EVENTS]);
    let events: Vec<_> = events.filter(|e| !e.is_empty()).collect();
    if !events.is_empty() {
        writeln!(file, "Recent events (memory in bytes):")?;
        for event in events {
            writeln!(file, "  {event}")?;
        }
        writeln!(file)?;
    }

    writeln!(file, "Backtrace:")?;
    for (i, &pc) in crash.backtrace.iter().enumerate() {
        writeln!(file, "  #{i:3}: {pc:08X}")?;
    }

    Ok(())
}

/// Save a previous crash to a file.
pub fn maybe_persist() {
    let crash = unsafe { REPORT.assume_init_mut() };
    if crash.magic == MAGIC_NUMBER {
        match write_crash(&crash) {
            Ok(()) => log::info!("Wrote crash log to sdcard"),
            Err(e) => log::warn!("Failed to write crash: {e}"),
        }
    }
    *crash = unsafe { std::mem::zeroed() };
}
