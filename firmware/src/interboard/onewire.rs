use embassy_executor::Spawner;
use embassy_futures::{
    select::{self, select},
    yield_now,
};
use embassy_rp::{
    clocks, dma::{self, AnyChannel}, peripherals::PIO0, pio::{Common, FifoJoin, Pin, PioPin, ShiftDirection, StateMachine}, Peripheral, PeripheralRef
};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, pipe::Pipe};
use embassy_time::{Duration, Timer};
use fixed::{traits::ToFixed, types::U56F8};

#[allow(unused_imports)]
use crate::utils::log;

pub static OTHER_SIDE_TX: Pipe<ThreadModeRawMutex, 32> = Pipe::new();
pub static OTHER_SIDE_RX: Pipe<ThreadModeRawMutex, 32> = Pipe::new();

pub const USART_SPEED: u64 = 460800;

pub fn init(
    spawner: &Spawner,
    common: &mut Common<'static, PIO0>,
    tx_sm: SM<0>,
    rx_sm: SM<1>,
    pin: impl Peripheral<P = impl PioPin + 'static> + 'static,
    dma: impl Peripheral<P = impl dma::Channel> + 'static,
) {
    let dma = dma.into_ref().map_into();
    let mut pin = common.make_pio_pin(pin);
    pin.set_pull(embassy_rp::gpio::Pull::Up);

    let tx_sm = half_duplex_task_tx(common, tx_sm, &mut pin);
    let rx_sm = half_duplex_task_rx(common, rx_sm, &pin);

    spawner.must_spawn(half_duplex_task(tx_sm, rx_sm, pin, dma));
}
pub type SM<const SM: usize> = StateMachine<'static, PIO0, { SM }>;

pub async fn enter_rx(tx_sm: &mut SM<0>, rx_sm: &mut SM<1>, pin: &mut Pin<'static, PIO0>) {
    while !tx_sm.tx().empty() {
        yield_now().await;
    }

    Timer::after(Duration::from_micros(1000000 * 11 / USART_SPEED)).await;

    tx_sm.set_enable(false);
    pin.set_drive_strength(embassy_rp::gpio::Drive::_2mA);
    tx_sm.set_pin_dirs(embassy_rp::pio::Direction::In, &[pin]);
    tx_sm.set_pins(embassy_rp::gpio::Level::High, &[pin]);
    rx_sm.restart();
    rx_sm.set_enable(true);
}

pub fn enter_tx(tx_sm: &mut SM<0>, rx_sm: &mut SM<1>, pin: &mut Pin<'static, PIO0>) {
    // okay so the way this works is that the pio doesn't actually set the pin
    // high or low, instead we toggle the input/output status of the pin to
    // switch it from a pull-high to a pull-low
    rx_sm.set_enable(false);
    rx_sm.set_pins(embassy_rp::gpio::Level::Low, &[pin]);
    pin.set_drive_strength(embassy_rp::gpio::Drive::_12mA);
    tx_sm.restart();
    tx_sm.set_enable(true);
}

#[embassy_executor::task]
pub async fn half_duplex_task(mut tx_sm: SM<0>,
                              mut rx_sm: SM<1>,
                              mut pin: Pin<'static, PIO0>,
                              mut dma: PeripheralRef<'static, AnyChannel>
) {
    enter_rx(&mut tx_sm, &mut rx_sm, &mut pin).await;

    let mut buf = [0u8; 16];
    let mut bbuf = [0u32; 16];

    loop {
        match select(OTHER_SIDE_TX.read(&mut buf), rx_sm.rx().wait_pull()).await {
            select::Either::First(n) => {
                // let now = Instant::now();
                // crate::log::info!("sending bytes: {:?}", &buf[..n]);
                for (n, x) in buf[..n].iter().enumerate() {
                    bbuf[n] = !*x as u32;
                }

                enter_tx(&mut tx_sm, &mut rx_sm, &mut pin);
                tx_sm.tx().dma_push(dma.reborrow(), &bbuf[..n]).await;
                enter_rx(&mut tx_sm, &mut rx_sm, &mut pin).await;
                // log::info!("sent bytes: {} in {}", &buf[..n], now.elapsed());
            }
            select::Either::Second(x) => {
                crate::set_status_led(embassy_rp::gpio::Level::High);
                let x = x.to_be_bytes()[0];
                // crate::log::info!("got byte: {:08b}: {}", 255 - x, 255 - x);
                OTHER_SIDE_RX.write(&[x]).await;

                while !OTHER_SIDE_RX.is_full() {
                    let Some(x) = rx_sm.rx().try_pull() else {
                        break;
                    };
                    let x = x.to_be_bytes()[0];
                    OTHER_SIDE_RX.write(&[x]).await;
                }
                crate::set_status_led(embassy_rp::gpio::Level::Low);
            }
        }
    }
}

fn pio_freq() -> fixed::FixedU32<fixed::types::extra::U8> {
    (U56F8::from_num(clocks::clk_sys_freq()) / (8 * USART_SPEED)).to_fixed()
}

pub fn half_duplex_task_tx(
    common: &mut Common<'static, PIO0>,
    mut sm: SM<0>,
    pin: &mut Pin<'static, PIO0>,
) -> SM<0> {
    let tx_prog = pio_proc::pio_asm!(
        ".wrap_target",
        "set   pindirs 0",
        "pull  block [6]",
        "set   pindirs 1",
        "set   x, 7  [6]",
        "bitloop:"
        "out   pindirs, 1",
        "jmp   x--, bitloop [6]",
        "set   pindirs, 0",
        ".wrap"
    );

    let mut cfg = embassy_rp::pio::Config::default();
    cfg.use_program(&common.load_program(&tx_prog.program), &[]);
    cfg.clock_divider = pio_freq();
    cfg.set_out_pins(&[pin]);
    cfg.set_set_pins(&[pin]);
    cfg.fifo_join = FifoJoin::TxOnly;
    cfg.shift_out.direction = ShiftDirection::Right;
    cfg.shift_in.auto_fill = false;
    sm.set_config(&cfg);
    sm.set_enable(true);

    pin.set_slew_rate(embassy_rp::gpio::SlewRate::Fast);
    pin.set_schmitt(true);

    sm
}

pub fn half_duplex_task_rx(
    common: &mut Common<'static, PIO0>,
    mut sm: SM<1>,
    pin: &Pin<'static, PIO0>,
) -> SM<1> {
    let rx_prog = pio_proc::pio_asm!(
        ".wrap_target",
        "start:",
        "wait  0 pin, 0",
        "set   x, 7         [10]",
        "bitloop:",
        "in    pins, 1",
        "jmp   x--, bitloop [6]",
        "jmp   pin, stop",
        "wait  1 pin, 0",
        "jmp   start",
        "stop:",
        "push block",
        ".wrap"
    );

    let mut cfg = embassy_rp::pio::Config::default();
    cfg.use_program(&common.load_program(&rx_prog.program), &[]);
    cfg.clock_divider = pio_freq();
    cfg.set_in_pins(&[pin]);
    cfg.set_jmp_pin(pin);
    cfg.shift_out.auto_fill = false;
    cfg.shift_in.direction = ShiftDirection::Right;
    cfg.fifo_join = FifoJoin::RxOnly;
    sm.set_config(&cfg);

    sm.set_enable(true);

    sm
}
