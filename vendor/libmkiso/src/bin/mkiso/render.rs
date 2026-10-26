use crate::args::ProgressMode;
use libmkiso::boot_media::progress::{Observer, ProgressEvent};

pub struct Renderer {
    #[cfg(feature = "progress")]
    bars: indicatif::MultiProgress,
    #[cfg(feature = "progress")]
    current: Option<indicatif::ProgressBar>,
    #[cfg(feature = "progress")]
    enabled: bool,
}
impl Renderer {
    pub fn new(mode: ProgressMode, json: bool) -> Self {
        #[cfg(feature = "progress")]
        {
            use std::io::IsTerminal;
            let enabled = matches!(mode, ProgressMode::Always)
                || (matches!(mode, ProgressMode::Auto) && !json && std::io::stderr().is_terminal());
            Self {
                bars: indicatif::MultiProgress::with_draw_target(
                    indicatif::ProgressDrawTarget::stderr_with_hz(8),
                ),
                current: None,
                enabled,
            }
        }
        #[cfg(not(feature = "progress"))]
        {
            let _ = (mode, json);
            Self {}
        }
    }
}
impl Observer for Renderer {
    fn observe(&mut self, event: &ProgressEvent) {
        #[cfg(feature = "progress")]
        {
            use libmkiso::boot_media::progress::ProgressState;
            if !self.enabled {
                return;
            }
            if matches!(event.state, ProgressState::Started) {
                if let Some(old) = self.current.take() {
                    old.finish_and_clear();
                }
                let bar = self.bars.add(match event.total {
                    Some(total) => indicatif::ProgressBar::new(total),
                    None => indicatif::ProgressBar::new_spinner(),
                });
                bar.set_message(format!("{:?} ({:?})", event.phase, event.unit));
                self.current = Some(bar);
            }
            if let Some(bar) = &self.current {
                bar.set_position(event.completed);
                bar.tick();
                if matches!(
                    event.state,
                    ProgressState::Finished | ProgressState::Failed | ProgressState::Cancelled
                ) {
                    bar.finish_and_clear();
                }
            }
        }
        #[cfg(not(feature = "progress"))]
        {
            let _ = event;
        }
    }
}
