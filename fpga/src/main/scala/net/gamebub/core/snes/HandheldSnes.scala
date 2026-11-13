package net.gamebub.core.snes

import chisel3._
import chisel3.util._
import HandheldSnes.CommandState
import lib.audio.AudioFilterChain
import lib.mem.{MemoryInterface, MemoryMap, RegisterMap}
import lib.video.ColorCorrection
import net.gamebub.framework.Core
import net.gamebub.framework.interface._
import xilinx.MMCM

object HandheldSnes {
  object CommandState extends ChiselEnum {
    val idle, busy, error, done = Value
  }

  /// MMCM VCO: 50 MHz / 2 * 34.375 = 859.375 MHz
  val mmcmVcoHz = 50_000_000.toDouble / 2 * 34.375

  /// SNES master clock (NTSC 21.477 MHz; this is 0.03% fast)
  val clockSystemHz = mmcmVcoHz / 40

  /// Master clock cycles per (non-interlaced NTSC) frame
  val cyclesPerFrame = 1364 * 262
}

/** The MiSTer SNES core with its memories (verilog/snes/snes_top.sv). */
class SnesTop extends BlackBox {
  override def desiredName = "snes_top"

  val io = IO(new Bundle {
    val clk_sys = Input(Clock())
    val clk_mem = Input(Clock())

    val reset = Input(Bool())
    val enable = Input(Bool())
    val sdram_init = Input(Bool())

    val rom_type = Input(UInt(8.W))
    val rom_mask = Input(UInt(24.W))
    val ram_mask = Input(UInt(24.W))
    val ram_size = Input(UInt(4.W))
    val pal = Input(Bool())

    val dl_en = Input(Bool())
    val dl_wr = Input(Bool())
    val dl_addr = Input(UInt(24.W))
    val dl_data = Input(UInt(16.W))

    val joy1 = Input(UInt(12.W))

    val video_r = Output(UInt(5.W))
    val video_g = Output(UInt(5.W))
    val video_b = Output(UInt(5.W))
    val video_valid = Output(Bool())
    val video_hblank = Output(Bool())
    val video_vblank = Output(Bool())

    val audio_l = Output(UInt(16.W))
    val audio_r = Output(UInt(16.W))

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

    val host_sram_req = Input(Bool())
    val host_sram_write = Input(Bool())
    val host_sram_addr = Input(UInt(18.W))
    val host_sram_wdata = Input(UInt(16.W))
    val host_sram_mask_n = Input(UInt(2.W))
    val host_sram_rdata = Output(UInt(16.W))
    val host_sram_ack = Output(Bool())

    val sram_addr = Output(UInt(18.W))
    val sram_dq_in = Input(UInt(16.W))
    val sram_dq_out = Output(UInt(16.W))
    val sram_dq_dir = Output(Bool())
    val sram_oe_n = Output(Bool())
    val sram_we_n = Output(Bool())
    val sram_mask_n = Output(UInt(2.W))
  })
}

class HandheldSnes extends Module with Core {
  val framePeriod = HandheldSnes.cyclesPerFrame / HandheldSnes.clockSystemHz
  val displayDivider = (HandheldSnes.mmcmVcoHz / ClocksV0.getClockDisplayHz(framePeriod)._1).floor.toInt
  val io = IO(new Bundle {
    val clocks = new ClocksV0(
      clockSystemHz = HandheldSnes.clockSystemHz.toInt,
      clockDisplayHz = (HandheldSnes.mmcmVcoHz / displayDivider).toInt,
      clockSpiHz = (HandheldSnes.mmcmVcoHz / 5).toInt,
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
      MMCM.ClockOut(40), // System (SNES master clock)
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

  // Cartridge configuration, from the ROM header
  val configRegRomType = RegInit(0.U(8.W))
  val configRegRomMask = RegInit(0.U(24.W))
  val configRegRamMask = RegInit(0.U(24.W))
  val configRegRamSize = RegInit(0.U(4.W))
  val configRegPal = RegInit(false.B)

  val registerInterface = Wire(new MemoryInterface(addressWidth = 16, dataWidth = 32))
  val sdramInterface = Wire(new MemoryInterface(addressWidth = 25, dataWidth = 32))
  val sramInterface = Wire(new MemoryInterface(addressWidth = 19, dataWidth = 32))
  val colorCorrectInterface = Wire(new MemoryInterface(addressWidth = 16, dataWidth = 16))
  val commandInterface = Wire(new MemoryInterface(addressWidth = 16, dataWidth = 32))
  val memoryMap = MemoryMap(
    addressWidth = 32,
    dataWidth = 32,
    entries = Seq(
      0x0.U(4.W) -> registerInterface,
      0x3.U(4.W) -> sdramInterface,
      0x4.U(4.W) -> sramInterface,
      0x5.U(4.W) -> colorCorrectInterface,
      0xF0.U(8.W) -> commandInterface,
    ))
  io.host.mem.unsafe :<>= memoryMap.unsafe
  memoryMap.writeStrobe := "b1111".U

  registerInterface <> RegisterMap(
    addressWidth = 16,
    dataWidth = 32,
    entries = Seq(
      0x0000 -> RegisterMap.Entry.rw(configRegRomType),
      0x0004 -> RegisterMap.Entry.rw(configRegRomMask),
      0x0008 -> RegisterMap.Entry.rw(configRegRamMask),
      0x000C -> RegisterMap.Entry.rw(configRegRamSize),
      0x0010 -> RegisterMap.Entry.rw(configRegPal),

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

  // SNES
  val snes = Module(new SnesTop)
  snes.io.clk_sys := clock
  snes.io.clk_mem := clockMem
  snes.io.reset := regCoreReset || regCoreResetOnce || reset.asBool
  snes.io.enable := regCoreFocus

  // Let the SDRAM power up for ~200 us before the controller initializes it.
  val sdramInitTimer = RegInit(0.U(13.W))
  when (!sdramInitTimer.andR) {
    sdramInitTimer := sdramInitTimer + 1.U
  }
  snes.io.sdram_init := !sdramInitTimer.andR

  snes.io.rom_type := configRegRomType
  snes.io.rom_mask := configRegRomMask
  snes.io.ram_mask := configRegRamMask
  snes.io.ram_size := configRegRamSize
  snes.io.pal := configRegPal

  // ROM download: each 32-bit host write becomes two 16-bit SDRAM writes,
  // a few cycles apart. Download mode is only held during the write, so
  // the SDRAM keeps being refreshed between host accesses.
  {
    val busy = RegInit(false.B)
    val timer = RegInit(0.U(4.W))
    val address = Reg(UInt(24.W))
    val data = Reg(UInt(32.W))

    sdramInterface.done := false.B
    sdramInterface.dataRead := 0.U
    snes.io.dl_en := busy
    snes.io.dl_wr := false.B
    snes.io.dl_addr := Cat(address(23, 2), timer >= 4.U, 0.U(1.W))
    snes.io.dl_data := Mux(timer >= 4.U, data(31, 16), data(15, 0))

    when (busy) {
      timer := timer + 1.U
      // Pulse the write on cycles 2 and 6 (snes_top registers the download
      // signals, and the SDRAM controller needs a few cycles per write).
      snes.io.dl_wr := timer === 2.U || timer === 6.U
      when (timer === 11.U) {
        busy := false.B
        sdramInterface.done := true.B
      }
    } .elsewhen (sdramInterface.enable) {
      when (sdramInterface.write) {
        busy := true.B
        timer := 0.U
        address := sdramInterface.address
        data := sdramInterface.dataWrite
      } .otherwise {
        sdramInterface.done := true.B
      }
    }
  }

  // Host SRAM access: each 32-bit access becomes two 16-bit accesses.
  {
    val busy = RegInit(false.B)
    val second = Reg(Bool())
    val write = Reg(Bool())
    val address = Reg(UInt(17.W)) // 32-bit word address
    val data = Reg(UInt(32.W))
    val readLow = Reg(UInt(16.W))

    snes.io.host_sram_req := busy
    snes.io.host_sram_write := write
    snes.io.host_sram_addr := Cat(address, second)
    snes.io.host_sram_wdata := Mux(second, data(31, 16), data(15, 0))
    snes.io.host_sram_mask_n := 0.U
    sramInterface.done := false.B
    sramInterface.dataRead := Cat(snes.io.host_sram_rdata, readLow)

    when (busy) {
      when (snes.io.host_sram_ack) {
        when (!second) {
          second := true.B
          readLow := snes.io.host_sram_rdata
        } .otherwise {
          busy := false.B
          sramInterface.done := true.B
        }
      }
    } .elsewhen (sramInterface.enable) {
      busy := true.B
      second := false.B
      write := sramInterface.write
      address := sramInterface.address(18, 2)
      data := sramInterface.dataWrite
    }
  }

  // Controller: {start, select, r, l, y, x, b, a, up, down, left, right}
  val buttons = io.input.buttons
  snes.io.joy1 := Cat(
    buttons.start, buttons.select, buttons.r, buttons.l,
    buttons.y, buttons.x, buttons.b, buttons.a,
    buttons.up, buttons.down, buttons.left, buttons.right,
  )

  // Video output
  io.video.data.r := snes.io.video_r
  io.video.data.g := snes.io.video_g
  io.video.data.b := snes.io.video_b
  io.video.dataEnable := snes.io.video_valid
  io.video.vblank := snes.io.video_vblank
  io.video.hblank := snes.io.video_hblank

  // Audio output and filtering (the DSP produces 32 kHz samples)
  val audioFilter = Module(new AudioFilterChain(
    inputRateHz = io.clocks.clockSystemHz,
    cicR = (io.clocks.clockSystemHz.toFloat / (4 * 48000)).ceil.toInt,
    dataBits = 16,
    outputRateHz = 48000,
  ))
  audioFilter.io.in(0) := snes.io.audio_l.asSInt
  audioFilter.io.in(1) := snes.io.audio_r.asSInt
  io.audio.left := audioFilter.io.out(0)
  io.audio.right := audioFilter.io.out(1)

  // SDRAM
  io.sdram.clock := mmcm.io.clockOuts(2)
  io.sdram.cke := snes.io.sdram_cke
  io.sdram.cs := snes.io.sdram_cs_n
  io.sdram.ras := snes.io.sdram_ras_n
  io.sdram.cas := snes.io.sdram_cas_n
  io.sdram.we := snes.io.sdram_we_n
  io.sdram.dqm := snes.io.sdram_dqm
  io.sdram.bank := snes.io.sdram_ba
  io.sdram.address := snes.io.sdram_a
  snes.io.sdram_dq_in := io.sdram.dataIn
  io.sdram.dataOut := snes.io.sdram_dq_out
  io.sdram.dataDir := snes.io.sdram_dq_dir

  // SRAM (BSRAM)
  io.sram.ceN := false.B
  io.sram.weN := snes.io.sram_we_n
  io.sram.oeN := snes.io.sram_oe_n
  io.sram.writeMaskN := snes.io.sram_mask_n
  io.sram.address := snes.io.sram_addr
  snes.io.sram_dq_in := io.sram.dataIn
  io.sram.dataOut := snes.io.sram_dq_out
  io.sram.dataDir := snes.io.sram_dq_dir

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
