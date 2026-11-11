//! What a command waits on while it runs: the core's events, Ctrl-C, and for the interactive
//! ones typed lines and a clock tick. Every command ends the same way on Ctrl-C and on the core
//! closing; each decides what that means for it.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader, Lines, Stdin};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Interval;

use crate::core::{ClientEvent, CoreEvent};

/// What came next.
pub enum Next {
    Event(CoreEvent),
    /// A typed line, trimmed.
    Line(String),
    Tick,
    /// Ctrl-C.
    Interrupted,
    /// The core closed.
    Closed,
    /// Standard input ended, or could not be read.
    InputEnded,
}

pub struct Wait<'a> {
    events: &'a mut UnboundedReceiver<ClientEvent>,
    interrupt: Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>,
    interrupted: bool,
    lines: Option<Lines<BufReader<Stdin>>>,
    ticks: Option<Interval>,
}

impl<'a> Wait<'a> {
    /// The core's events and Ctrl-C.
    pub fn new(events: &'a mut UnboundedReceiver<ClientEvent>) -> Wait<'a> {
        Wait {
            events,
            interrupt: Box::pin(tokio::signal::ctrl_c()),
            interrupted: false,
            lines: None,
            ticks: None,
        }
    }

    /// Also the lines typed on standard input.
    pub fn typed(mut self) -> Self {
        self.lines = Some(BufReader::new(tokio::io::stdin()).lines());
        self
    }

    /// Also a tick every `period`.
    pub fn ticking(mut self, period: Duration) -> Self {
        self.ticks = Some(tokio::time::interval(period));
        self
    }

    pub async fn next(&mut self) -> Next {
        if self.interrupted {
            return Next::Interrupted;
        }
        let Wait {
            events,
            interrupt,
            interrupted,
            lines,
            ticks,
        } = self;
        let line = async {
            match lines {
                Some(lines) => lines.next_line().await,
                None => std::future::pending().await,
            }
        };
        let tick = async {
            match ticks {
                Some(ticks) => ticks.tick().await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(line, tick);
        loop {
            tokio::select! {
                _ = &mut *interrupt => {
                    *interrupted = true;
                    return Next::Interrupted;
                }
                event = events.recv() => match event {
                    None | Some(ClientEvent::Closed) => return Next::Closed,
                    Some(ClientEvent::Event(event)) => return Next::Event(event),
                    Some(ClientEvent::Reconnected) => continue,
                },
                _ = &mut tick => return Next::Tick,
                line = &mut line => return match line {
                    Ok(Some(line)) => Next::Line(line.trim().to_string()),
                    _ => Next::InputEnded,
                },
            }
        }
    }
}
