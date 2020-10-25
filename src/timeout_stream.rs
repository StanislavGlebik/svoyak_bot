
use std::mem;

use failure::{err_msg, Error};
use futures::{Async, Future, Poll, Stream};
use futures::sync::mpsc::Receiver;

pub struct TimeoutStream {
    new_timers_stream: Receiver<Option<Box<dyn Future<Item = (), Error = Error>>>>,
    inflight_timer: Option<Box<dyn Future<Item = (), Error = Error>>>,
}

impl TimeoutStream {
    pub fn new(new_timers_stream: Receiver<Option<Box<dyn Future<Item = (), Error = Error>>>>) -> Self {
        Self {
            new_timers_stream,
            inflight_timer: None,
        }
    }
}

impl Stream for TimeoutStream {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            let new_timer = self.new_timers_stream.poll().map_err(|_| {
                err_msg("sending timer failed")
            })?;
            match new_timer {
                Async::Ready(Some(timer_or_cancel)) => {
                    match timer_or_cancel {
                        Some(timer) => {
                            let fut = Box::new(timer.map_err(|err| {
                                let msg = format!("timer failed: {}", err);
                                err_msg(msg)
                            }));
                            let _ = mem::replace(&mut self.inflight_timer, Some(fut));
                        }
                        None => {
                            let _ = mem::replace(&mut self.inflight_timer, None);
                        }
                    }
                }
                Async::NotReady |
                Async::Ready(None) => {
                    break;
                }
            }
        }

        let res = match self.inflight_timer {
            Some(ref mut timer) => {
                match timer.poll()? {
                    Async::Ready(_) => Async::Ready(Some(())),
                    Async::NotReady => {
                        return Ok(Async::NotReady);
                    }
                }
            }
            None => {
                return Ok(Async::NotReady);
            }
        };

        let _ = mem::replace(&mut self.inflight_timer, None);
        Ok(res)
    }
}
