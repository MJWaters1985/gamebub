package net.gamebub.core.nes

import chisel3._
import chisel3.util._
import HandheldNes.CommandState
import lib.audio.AudioFilterChain
import lib.mem.{MemoryInterface, MemoryMap, RegisterMap}
import lib.video.ColorCorrection
import net.gamebub.framework.Core
import net.gamebub.framework.interface._
import xilinx.MMCM

object HandheldNes {
  object CommandState extends ChiselEnum {
    val idle, busy, error, done = Value
  }

  /// MMCM VCO: 50 MHz / 2 * 34.375 = 859.375 MHz
  val mmcmVcoHz = 50_000_000.toDouble / 2 * 34.375

  /// NES master clock (NTSC 21.477 MHz; this is 0.03% fast)
  val clockSystemHz = mmcmVcoHz / 40

  /// Master clock cycles per NTSC frame (341 x 262 dots, 4 cycles each, one
  /// dot skipped every other frame)
  val cyclesPerFrame = 341 * 262 * 4 - 2
}

/** The MiSTer NES core with its memories (verilog/nes/nes_top.sv). */
class NesTop extends BlackBox {
  override def desiredName = "nes_top"

  val io = IO(new Bundle {
    val clk_sys = Input(Clock())
    val clk_mem = Input(Clock())

    val reset = Input(Bool())
    val enable = Input(Bool())
    val sdram_init = Input(Bool())
    val sys_type = Input(UInt(2.W))

    val dl_en = Input(Bool())
    val dl_type = Input(UInt(2.W))
    val dl_wr = Input(Bool())
    val dl_data = Input(UInt(8.W))
    val loader_busy = Output(Bool())
    val loader_fail = Output(Bool())

    val save_req = Input(Bool())
    val save_write = Input(Bool())
    val save_addr = Input(UInt(18.W))
    val save_wdata = Input(UInt(8.W))
    val save_rdata = Output(UInt(8.W))
    val save_ack = Output(Bool())

    val joy1 = Input(UInt(8.W))

    val video_r = Output(UInt(5.W))
    val video_g = Output(UInt(5.W))
    val video_b = Output(UInt(5.W))
    val video_valid = Output(Bool())
    val video_hblank = Output(Bool())
    val video_vblank = Output(Bool())

    val audio = Output(UInt(16.W))

    val sdram_dq_in = Input(UInt(16.W))
    val sdram_dq_out = Output(UInt(16.W))
    val sdram_dq_dir = Output(Bool())
    val sdram_a = Output(UInt(13.W))
    val sdram_dqm = Output(UInt(2.W))
    val sdram_ba = Output(UInt(2.W))
    val sdram_cs_n = Output(Bool())
    val sdram_we_n = Output(Bool())
    val sdram_ras_n = Output(Bool())
    val sdram_cas_n = Output(Bool())
    val sdram_cke = Output(Bool())
  })
}

class HandheldNes extends Module with Core {
  val framePeriod = HandheldNes.cyclesPerFrame / HandheldNes.clockSystemHz
  val displayDivider = (HandheldNes.mmcmVcoHz / ClocksV0.getClockDisplayHz(framePeriod)._1).floor.toInt
  val io = IO(new Bundle {
    val clocks = new ClocksV0(
      clockSystemHz = HandheldNes.clockSystemHz.toInt,
      clockDisplayHz = (HandheldNes.mmcmVcoHz / displayDivider).toInt,
      clockSpiHz = (HandheldNes.mmcmVcoHz / 5).toInt,
    )
    val video = new VideoV0(
      videoWidth = 256,
      videoHeight = 224,
      colorDepthR = 5,
      colorDepthG = 5,
      colorDepthB = 5,
      framePeriod = framePeriod,
    )
    val videoFilter = new VideoFilterBasicV0(
      colorInDepthR = 5,
      colorInDepthG = 5,
      colorInDepthB = 5,
      latency = 3,
    )
    val audio = new AudioV0()
    val host = new HostV0()
    val pmod = new PmodV0()
    val input = new InputV0()
    val vibrate = new VibrateV0()
    val cartridge = new CartridgePortV0()
    val link = new LinkPortV0()
    val sram = new SramV0()
    val sdram = new SdramV0()
  })

  // Main MMCM
  val mmcm = Module(new MMCM(
    clockInHz = 50_000_000,
    divide = 2,
    multiply = 34.375,
    clockOutConfig = Seq(
      MMCM.ClockOut(40), // System (NES master clock)
      MMCM.ClockOut(10), // SDRAM controller (4x)
      MMCM.ClockOut(10, phase = 180.0), // SDRAM chip clock
      MMCM.ClockOut(displayDivider), // Display
      MMCM.ClockOut(5), // Host SPI
    )
  ))
  mmcm.io.clockIn := io.clocks.clockIn50M
  mmcm.io.powerDown := false.B
  io.clocks.clockOutSystem := mmcm.io.clockOuts(0)
  val clockMem = mmcm.io.clockOuts(1)
  io.clocks.clockOutDisplay := mmcm.io.clockOuts(3)
  io.clocks.clockOutSpi := mmcm.io.clockOuts(4)
  io.clocks.locked := mmcm.io.locked

  val regCoreSetup = RegInit(false.B)
  val regCoreReset = RegInit(true.B)
  val regCoreFocus = RegInit(false.B)
  val regCoreResetOnce = RegInit(false.B)
  regCoreResetOnce := false.B

  // Loading
  val regDownloadType = RegInit(0.U(2.W))
  val regDownload = RegInit(false.B)
  val regSysType = RegInit(0.U(2.W))

  val nes = Module(new NesTop)

  val registerInterface = Wire(new MemoryInterface(addressWidth = 16, dataWidth = 32))
  val downloadInterface = Wire(new MemoryInterface(addressWidth = 25, dataWidth = 32))
  val saveInterface = Wire(new MemoryInterface(addressWidth = 19, dataWidth = 32))
  val colorCorrectInterface = Wire(new MemoryInterface(addressWidth = 16, dataWidth = 16))
  val commandInterface = Wire(new MemoryInterface(addressWidth = 16, dataWidth = 32))
  val memoryMap = MemoryMap(
    addressWidth = 32,
    dataWidth = 32,
    entries = Seq(
      0x0.U(4.W) -> registerInterface,
      0x3.U(4.W) -> downloadInterface,
      0x4.U(4.W) -> saveInterface,
      0x5.U(4.W) -> colorCorrectInterface,
      0xF0.U(8.W) -> commandInterface,
    ))
  io.host.mem.unsafe :<>= memoryMap.unsafe
  memoryMap.writeStrobe := "b1111".U

  val loaderStatus = Cat(nes.io.loader_fail, nes.io.loader_busy)
  registerInterface <> RegisterMap(
    addressWidth = 16,
    dataWidth = 32,
    entries = Seq(
      0x0000 -> RegisterMap.Entry.rw(regDownloadType),
      0x0004 -> RegisterMap.Entry.rw(regDownload),
      0x0008 -> RegisterMap.Entry.r(loaderStatus),
      0x000C -> RegisterMap.Entry.rw(regSysType),

      0x2000 -> RegisterMap.Entry.w(regCoreResetOnce),
    )
  )

  // Command interface
  val commandHostState = RegInit(CommandState.idle)
  val regCommandHost = Reg(Vec(2, UInt(32.W)))
  commandInterface <> RegisterMap(
    addressWidth = 16,
    dataWidth = 32,
    entries =
      regCommandHost.zipWithIndex.map { case (reg, i) => (0x0000 + (4 * i) -> RegisterMap.Entry.rw(reg)) }
  )
  // Host -> Core commands
  io.host.commandHost.busy := commandHostState === CommandState.busy
  io.host.commandHost.done := commandHostState === CommandState.done
  io.host.commandHost.error := commandHostState === CommandState.error
  when (io.host.commandHost.request) {
    when (commandHostState === CommandState.idle) {
      val command = regCommandHost(0)(15, 0)
      for (reg <- regCommandHost) {
        reg := 0.U
      }
      commandHostState := CommandState.done

      when (command === HostV0.CommandGetStatus.U) {
        when (regCoreSetup) {
          regCommandHost(0) := Mux(regCoreReset, HostV0.StatusCoreHalt.U, HostV0.StatusCoreRun.U)
        } .otherwise {
          regCommandHost(0) := HostV0.StatusSetup.U
        }
      } .elsewhen (command === HostV0.CommandSetupComplete.U) {
        regCoreSetup := true.B
      } .elsewhen (command === HostV0.CommandCoreRun.U) {
        regCoreReset := false.B
      } .elsewhen (command === HostV0.CommandCoreHalt.U) {
        regCoreReset := true.B
      } .elsewhen (command === HostV0.CommandNotifyFocus.U) {
        regCoreFocus := regCommandHost(1)(0)
      } .elsewhen (command(15, 8) === 0x03.U) {
        // File command
      } .otherwise {
        commandHostState := CommandState.error
      }
    }
  } .otherwise {
    commandHostState := CommandState.idle
  }
  // Core -> Host commands
  io.host.commandCore.request := false.B

  io.vibrate.mode := VibrateV0.Mode.Off

  // NES
  nes.io.clk_sys := clock
  nes.io.clk_mem := clockMem
  // The console stays in reset until the host runs it (games load while it's
  // halted; the loader isn't affected by this reset).
  nes.io.reset := regCoreReset || regCoreResetOnce || reset.asBool
  nes.io.enable := regCoreFocus
  nes.io.sys_type := regSysType

  // Let the SDRAM power up for ~200 us before the controller initializes it.
  val sdramInitTimer = RegInit(0.U(13.W))
  when (!sdramInitTimer.andR) {
    sdramInitTimer := sdramInitTimer + 1.U
  }
  nes.io.sdram_init := !sdramInitTimer.andR

  // Game download: each 32-bit host write becomes four bytes for the loader
  // (file order, so little-endian), 4 cycles apart: 5.4 MB/s, a little more
  // than the host's slowest SPI clock delivers.
  {
    val busy = RegInit(false.B)
    val timer = RegInit(0.U(4.W))
    val data = Reg(UInt(32.W))

    downloadInterface.done := false.B
    downloadInterface.dataRead := 0.U
    nes.io.dl_en := regDownload
    nes.io.dl_type := regDownloadType
    nes.io.dl_wr := busy && timer(1, 0) === 0.U
    nes.io.dl_data := (data >> (timer(3, 2) ## 0.U(3.W)))(7, 0)

    when (busy) {
      timer := timer + 1.U
      when (timer === 15.U) {
        busy := false.B
        downloadInterface.done := true.B
      }
    } .elsewhen (downloadInterface.enable) {
      when (downloadInterface.write) {
        busy := true.B
        timer := 0.U
        data := downloadInterface.dataWrite
      } .otherwise {
        downloadInterface.done := true.B
      }
    }
  }

  // Save RAM: each 32-bit host access becomes four byte accesses.
  {
    val busy = RegInit(false.B)
    val byte = Reg(UInt(2.W))
    val write = Reg(Bool())
    val address = Reg(UInt(16.W)) // 32-bit word address
    val data = Reg(UInt(32.W))
    val readData = Reg(Vec(4, UInt(8.W)))

    nes.io.save_req := busy
    nes.io.save_write := write
    nes.io.save_addr := Cat(address, byte)
    nes.io.save_wdata := (data >> (byte ## 0.U(3.W)))(7, 0)
    saveInterface.done := false.B
    saveInterface.dataRead := readData.asUInt

    when (busy) {
      when (nes.io.save_ack) {
        readData(byte) := nes.io.save_rdata
        byte := byte + 1.U
        when (byte === 3.U) {
          busy := false.B
          saveInterface.done := true.B
        }
      }
    } .elsewhen (saveInterface.enable) {
      busy := true.B
      byte := 0.U
      write := saveInterface.write
      address := saveInterface.address(17, 2)
      data := saveInterface.dataWrite
    }
  }

  // Controller: {right, left, down, up, start, select, b, a}
  val buttons = io.input.buttons
  nes.io.joy1 := Cat(
    buttons.right, buttons.left, buttons.down, buttons.up,
    buttons.start, buttons.select, buttons.b, buttons.a,
  )

  // Video output
  io.video.data.r := nes.io.video_r
  io.video.data.g := nes.io.video_g
  io.video.data.b := nes.io.video_b
  io.video.dataEnable := nes.io.video_valid
  io.video.vblank := nes.io.video_vblank
  io.video.hblank := nes.io.video_hblank

  // Audio output and filtering (mono)
  val audioFilter = Module(new AudioFilterChain(
    inputRateHz = io.clocks.clockSystemHz,
    cicR = (io.clocks.clockSystemHz.toFloat / (4 * 48000)).ceil.toInt,
    dataBits = 16,
    outputRateHz = 48000,
  ))
  audioFilter.io.in(0) := nes.io.audio.asSInt
  audioFilter.io.in(1) := nes.io.audio.asSInt
  io.audio.left := audioFilter.io.out(0)
  io.audio.right := audioFilter.io.out(1)

  // SDRAM
  io.sdram.clock := mmcm.io.clockOuts(2)
  io.sdram.cke := nes.io.sdram_cke
  io.sdram.cs := nes.io.sdram_cs_n
  io.sdram.ras := nes.io.sdram_ras_n
  io.sdram.cas := nes.io.sdram_cas_n
  io.sdram.we := nes.io.sdram_we_n
  io.sdram.dqm := nes.io.sdram_dqm
  io.sdram.bank := nes.io.sdram_ba
  io.sdram.address := nes.io.sdram_a
  nes.io.sdram_dq_in := io.sdram.dataIn
  io.sdram.dataOut := nes.io.sdram_dq_out
  io.sdram.dataDir := nes.io.sdram_dq_dir

  // SRAM (unused)
  io.sram.ceN := true.B
  io.sram.weN := true.B
  io.sram.oeN := true.B
  io.sram.writeMaskN := "b11".U
  io.sram.address := 0.U
  io.sram.dataOut := 0.U
  io.sram.dataDir := false.B

  // Unused peripherals
  io.pmod.out := 0.U
  io.pmod.dir := 0.U
  io.link.scOut := false.B
  io.link.sdOut := false.B
  io.link.siOut := false.B
  io.link.soOut := false.B
  io.link.scDir := false.B
  io.link.sdDir := false.B
  io.link.siDir := false.B
  io.link.soDir := false.B
  io.cartridge.enabled := false.B
  io.cartridge.bank0Out := DontCare
  io.cartridge.bank1Out := DontCare
  io.cartridge.bank2Out := DontCare
  io.cartridge.bank3Out := DontCare
  io.cartridge.pin30Out := DontCare
  io.cartridge.pin31Out := DontCare
  io.cartridge.bank0Dir := false.B
  io.cartridge.bank1Dir := false.B
  io.cartridge.bank2Dir := false.B
  io.cartridge.bank3Dir := false.B
  io.cartridge.pin30Dir := false.B
  io.cartridge.pin31Dir := false.B

  // Video filter (color correction)
  ColorCorrection.setup(
    clock = clock,
    reset = reset,
    videoFilter = io.videoFilter,
    memInterface = colorCorrectInterface,
  )
}
