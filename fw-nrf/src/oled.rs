//! The OLED: keeps this half's screen in sync with the status the key loop
//! publishes (`SCREEN`), the battery level and diagnostic messages, blanking
//! it when idle and animating it while something on it moves.

use embassy_futures::select::{Either4, select4};
use embassy_nrf::twim;
use embassy_time::{Duration, Instant, Timer};
use kb_core::keymaps::corne;
use kb_core::power::PowerState;
use kb_display::anim::{Animator, SLEEP_ART_MS, SLEEP_FRAME_MS};
use kb_display::frame::Frame;
use kb_display::screens::{self, Style};
use kb_display::ssd1306;
use kb_display::status::Status;

#[cfg(all(feature = "usb-log", not(all(feature = "ble", feature = "right"))))]
use crate::logport;
use crate::{BATTERY, SCREEN, Screen, diag, matrix, sleep};

/// Redraw interval of the start-up animation (25 frames a second).
const BOOT_FRAME_MS: u32 = 40;

/// How soon a frame the OLED didn't take is sent again.
const OLED_RETRY_MS: u32 = 200;

/// Keeps the OLED in sync with the status, blanking it when idle. While
/// something on the screen moves (`kb_display::anim`), it redraws on its own
/// timer too.
#[embassy_executor::task]
pub async fn display_task(i2c: twim::Twim<'static>) -> ! {
    let mut boot = Boot::new(Instant::now().as_millis() as u32);
    // The OLED gets its power at the same moment as the chip (power switch):
    // give it a moment before the first command, or it misses the init.
    Timer::after(Duration::from_millis(100)).await;
    let mut oled = Oled {
        i2c,
        shown: None,
        lit: false,
    };
    let mut last: Option<Screen> = None;
    let mut latest: Option<Screen> = None;
    let mut battery_pct = 100u8;
    let mut animator = Animator::new();
    // When the keyboard started resting (for the sleep picture).
    let mut idle_at: Option<u32> = None;
    // When the screen next changes by itself (milliseconds), if it moves.
    let mut next_frame: Option<u32> = None;
    loop {
        // What the OLED shows now, for the log port's debug stream.
        #[cfg(all(feature = "usb-log", not(all(feature = "ble", feature = "right"))))]
        logport::show_frame(oled.shown.as_ref().filter(|_| oled.lit));
        // The key loop sends the status, the battery task the battery level;
        // whichever arrives, the screen is redrawn with the latest of both.
        let tick = async {
            match next_frame {
                Some(ms) => Timer::after_millis(u64::from(ms)).await,
                None => core::future::pending().await,
            }
        };
        let event = select4(SCREEN.wait(), BATTERY.wait(), diag::MESSAGE.wait(), tick).await;
        let animating = next_frame.is_some();
        let mut screen = match event {
            Either4::First(screen) => {
                latest = Some(screen);
                screen
            }
            Either4::Second(pct) => {
                battery_pct = pct;
                match latest {
                    Some(screen) => screen,
                    None => continue,
                }
            }
            Either4::Third((title, text)) => {
                // Diagnostics: draw right away, whatever the power state.
                oled.show_message(title, text.as_str()).await;
                last = None;
                next_frame = None;
                continue;
            }
            Either4::Fourth(()) => {
                if let Some(screen) = latest {
                    screen
                } else {
                    next_frame = None;
                    continue;
                }
            }
        };
        screen.status.battery[matrix::HALF as usize] = battery_pct;
        screen.status.charging[matrix::HALF as usize] = sleep::usb_powered();
        // The loop signals every 50 ms; only render when something changed
        // or moves.
        if last == Some(screen) && !animating {
            continue;
        }
        last = Some(screen);
        match screen.power {
            PowerState::Sleep => {
                next_frame = None;
                idle_at = None;
                continue;
            }
            PowerState::Idle => {
                next_frame = oled.show_idle(&screen.status, &mut idle_at).await;
                continue;
            }
            PowerState::Active => idle_at = None,
        }

        let now = Instant::now().as_millis() as u32;
        let frame;
        (frame, next_frame) = picture(&mut animator, &mut boot, &screen.status, now);
        if !oled.show(frame).await {
            // Not acknowledged (the OLED still powering up, or a failed
            // write): start the OLED over soon.
            next_frame = Some(next_frame.map_or(OLED_RETRY_MS, |ms| ms.min(OLED_RETRY_MS)));
            last = None;
        }
    }
}

/// The frame for `status` at `now`, and when it next changes by itself.
fn picture(
    animator: &mut Animator,
    boot: &mut Boot,
    status: &Status,
    now: u32,
) -> (Frame, Option<u32>) {
    let mut anim = animator.update(status, now);
    if boot.running {
        anim = animator.starting_up();
    }
    let mut next_frame = animator.next_frame(matrix::HALF, status);
    let mut frame = Frame::default();
    screens::render_animated(
        &mut frame,
        matrix::HALF,
        Style::Portrait,
        status,
        corne::KEYMAP.layer_names,
        &anim,
    );
    if boot.draw(&mut frame, status.peripheral_connected, now) {
        next_frame = Some(BOOT_FRAME_MS);
    }
    (frame, next_frame)
}

/// The start-up animation: it runs from power-on until both halves play it
/// together (`kb_display::screens::boot_frame`).
struct Boot {
    /// Power-on (milliseconds).
    start: u32,
    running: bool,
    /// When the halves linked, while they are.
    linked_at: Option<u32>,
    /// The letter this half shows.
    initial: &'static str,
}

impl Boot {
    fn new(start: u32) -> Self {
        Self {
            start,
            running: true,
            linked_at: None,
            initial: match matrix::HALF {
                kb_display::frame::Half::Left => "E",
                kb_display::frame::Half::Right => "L",
            },
        }
    }

    /// Start-up: rain until the halves are linked (with a hint to switch the
    /// other half on), then the initial and the reveal on both together,
    /// drawn over the status `frame`. Whether it is still running.
    fn draw(&mut self, frame: &mut Frame, linked: bool, now: u32) -> bool {
        if !self.running {
            return false;
        }
        if linked {
            self.linked_at.get_or_insert(now);
        } else {
            self.linked_at = None;
        }
        let since = now.wrapping_sub(self.start);
        let since_linked = self.linked_at.map(|t| now.wrapping_sub(t));
        if screens::boot_done(since, since_linked) {
            self.running = false;
            return false;
        }
        let status_frame = frame.clone();
        screens::boot_frame(
            frame,
            matrix::HALF,
            since,
            since_linked,
            self.initial,
            &status_frame,
        );
        true
    }
}

/// This half's OLED: the I²C bus to it and what it shows.
struct Oled {
    i2c: twim::Twim<'static>,
    /// The frame on the panel; `None` until it is set up, and again after a
    /// failed write.
    shown: Option<Frame>,
    /// Whether the panel is switched on.
    lit: bool,
}

impl Oled {
    /// One I²C transaction; whether the OLED acknowledged it.
    async fn send(&mut self, bytes: &[u8]) -> bool {
        self.i2c.write(ssd1306::ADDRESS, bytes).await.is_ok()
    }

    /// The power-up sequence; whether all of it was acknowledged.
    async fn init(&mut self) -> bool {
        let mut ok = true;
        for transaction in ssd1306::INIT {
            ok &= self.send(transaction).await;
        }
        ok
    }

    /// The address window and the pixels; whether both were acknowledged.
    async fn write_frame(&mut self, frame: &Frame) -> bool {
        let header = self.send(&ssd1306::FLUSH_HEADER).await;
        let data = self.send(&ssd1306::frame_data(frame)).await;
        header && data
    }

    /// A diagnostic message, whatever the power state.
    async fn show_message(&mut self, title: &str, text: &str) {
        let mut frame = Frame::default();
        screens::message(&mut frame, matrix::HALF, title, text);
        if self.shown.is_none() {
            self.init().await;
        }
        self.lit = true;
        self.write_frame(&frame).await;
        self.shown = Some(frame);
    }

    /// Resting: the sleep picture for a moment (from `idle_at`, set on the
    /// first call), then off. When the picture next changes, if it is up.
    async fn show_idle(&mut self, status: &Status, idle_at: &mut Option<u32>) -> Option<u32> {
        if !self.lit {
            return None;
        }
        let now = Instant::now().as_millis() as u32;
        let elapsed = now.wrapping_sub(*idle_at.get_or_insert(now));
        if elapsed < SLEEP_ART_MS {
            let mut frame = Frame::default();
            let mut from = Frame::default();
            screens::render(
                &mut from,
                matrix::HALF,
                Style::Portrait,
                status,
                corne::KEYMAP.layer_names,
            );
            screens::sleep_art(&mut frame, matrix::HALF, elapsed, &from);
            if self.shown.as_ref() != Some(&frame) {
                self.write_frame(&frame).await;
                self.shown = Some(frame);
            }
            return Some(SLEEP_FRAME_MS);
        }
        self.lit = false;
        self.send(&ssd1306::DISPLAY_OFF).await;
        None
    }

    /// Show `frame`, setting the OLED up or switching it on first. Whether
    /// it got there; if not, the OLED is set up again with the next frame.
    async fn show(&mut self, frame: Frame) -> bool {
        if self.shown.is_none() {
            if !self.init().await {
                return false;
            }
        } else if !self.lit {
            self.send(&ssd1306::DISPLAY_ON).await;
        }
        self.lit = true;
        if self.shown.as_ref() != Some(&frame) {
            if !self.write_frame(&frame).await {
                self.shown = None;
                return false;
            }
            self.shown = Some(frame);
        }
        true
    }
}
