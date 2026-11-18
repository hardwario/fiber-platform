//! The buzzer (`docs/mqtt-api.md`, `buzzer.beep`): it sounds while GPIO17 is low, at its own
//! tone (about 2.4 kHz), so a pattern is only when it sounds and when not.

use std::time::Duration;

/// A pattern of the buzzer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Beep,
    Double,
    Long,
    Alarm,
}

impl Pattern {
    pub fn parse(name: &str) -> Option<Pattern> {
        Some(match name {
            "beep" => Pattern::Beep,
            "double" => Pattern::Double,
            "long" => Pattern::Long,
            "alarm" => Pattern::Alarm,
            _ => return None,
        })
    }

    /// The pattern as (sound, then silence) steps.
    pub fn steps(self) -> Vec<(Duration, Duration)> {
        let ms = Duration::from_millis;
        match self {
            Pattern::Beep => vec![(ms(100), ms(0))],
            Pattern::Double => vec![(ms(100), ms(100)), (ms(100), ms(0))],
            Pattern::Long => vec![(ms(500), ms(0))],
            Pattern::Alarm => {
                let mut steps = vec![(ms(150), ms(100)); 5];
                steps[4].1 = ms(0);
                steps
            }
        }
    }
}

/// The buzzer's line.
pub trait Line {
    fn sound(&mut self, on: bool);
}

/// Plays the patterns that come in, one at a time; a new one cuts the one that sounds short.
/// Ends silent when `patterns` is closed.
pub async fn play<L: Line>(mut line: L, mut patterns: tokio::sync::mpsc::UnboundedReceiver<Pattern>) {
    let mut next = patterns.recv().await;
    while let Some(pattern) = next.take() {
        {
            let steps = async {
                for (on, off) in pattern.steps() {
                    line.sound(true);
                    tokio::time::sleep(on).await;
                    line.sound(false);
                    tokio::time::sleep(off).await;
                }
            };
            tokio::pin!(steps);
            tokio::select! {
                () = &mut steps => next = patterns.recv().await,
                other = patterns.recv() => match other {
                    Some(pattern) => next = Some(pattern),
                    // Nothing more will come: this one sounds to its end.
                    None => steps.await,
                },
            }
        }
        line.sound(false);
    }
    line.sound(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::time::Instant;

    /// A line that writes down when it changed.
    #[derive(Clone)]
    struct Recorded(Arc<Mutex<Vec<(u64, bool)>>>, Instant);

    impl Line for Recorded {
        fn sound(&mut self, on: bool) {
            let mut seen = self.0.lock().unwrap();
            if seen.last().map(|(_, was)| *was) != Some(on) {
                seen.push((self.1.elapsed().as_millis() as u64, on));
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn patterns_as_they_sound() {
        let line = Recorded(Arc::new(Mutex::new(vec![(0, false)])), Instant::now());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let player = tokio::spawn(play(line.clone(), rx));
        tx.send(Pattern::Double).unwrap();
        tokio::time::sleep(Duration::from_millis(1000)).await;
        // A long one, cut short by a beep after 200 ms.
        tx.send(Pattern::Long).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        tx.send(Pattern::Beep).unwrap();
        drop(tx);
        player.await.unwrap();
        assert_eq!(
            *line.0.lock().unwrap(),
            [
                (0, false),
                (0, true),
                (100, false),
                (200, true),
                (300, false),
                (1000, true),
                (1200, false),
                (1200, true),
                (1300, false)
            ]
        );
    }

    #[test]
    fn names() {
        assert_eq!(Pattern::parse("alarm"), Some(Pattern::Alarm));
        assert_eq!(Pattern::parse("siren"), None);
        assert_eq!(Pattern::Alarm.steps().len(), 5);
    }
}
