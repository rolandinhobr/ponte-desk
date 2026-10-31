use async_trait::async_trait;
use core::task::{Context, Poll};
use event_thread::EventThread;
use futures::Stream;
use std::pin::Pin;

use input_event::{BTN_LEFT, Event, PointerEvent};
use tokio::sync::mpsc::{Receiver, channel};

use super::{Capture, CaptureCreationError, CaptureError, CaptureEvent, Position};

mod display_util;
mod event_thread;

pub struct WindowsInputCapture {
    // Stop callbacks and join their thread before closing the event consumer.
    event_thread: EventThread,
    event_rx: Receiver<(Position, CaptureEvent)>,
    delivered_buttons: [Option<Position>; 5],
}

#[async_trait]
impl Capture for WindowsInputCapture {
    fn direct(&mut self, pos: Option<Position>) -> Result<(), CaptureError> {
        self.event_thread.direct(pos);
        Ok(())
    }
    async fn create(&mut self, pos: Position) -> Result<(), CaptureError> {
        self.event_thread.create(pos);
        Ok(())
    }

    async fn destroy(&mut self, pos: Position) -> Result<(), CaptureError> {
        self.event_thread.destroy(pos);
        Ok(())
    }

    async fn release(&mut self) -> Result<(), CaptureError> {
        self.event_thread.release_capture();
        Ok(())
    }

    async fn terminate(&mut self) -> Result<(), CaptureError> {
        Ok(())
    }
}

impl WindowsInputCapture {
    pub(crate) fn new() -> Result<Self, CaptureCreationError> {
        let (event_tx, event_rx) = channel(128);
        let event_thread = EventThread::new(event_tx)?;
        Ok(Self {
            event_thread,
            event_rx,
            delivered_buttons: [None; 5],
        })
    }
}

impl Stream for WindowsInputCapture {
    type Item = Result<(Position, CaptureEvent), CaptureError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.event_rx.poll_recv(cx) {
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some((pos, event))) => {
                if let CaptureEvent::Input(Event::Pointer(PointerEvent::Button {
                    button,
                    state,
                    ..
                })) = event
                {
                    if let Some(index) = button.checked_sub(BTN_LEFT).filter(|i| *i < 5) {
                        self.delivered_buttons[index as usize] =
                            if state == 1 { Some(pos) } else { None };
                    }
                }
                Poll::Ready(Some(Ok((pos, event))))
            }
            Poll::Pending => {
                // Only repair releases after all queued transitions have drained.
                // The physical mask is updated by non-software hook events even
                // when the bounded queue cannot accept their button-up.
                let physical = self.event_thread.physical_buttons();
                if let Some((pos, event)) = repair_release(&mut self.delivered_buttons, physical) {
                    Poll::Ready(Some(Ok((pos, event))))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}
fn repair_release(
    delivered: &mut [Option<Position>; 5],
    physical: u32,
) -> Option<(Position, CaptureEvent)> {
    for (index, slot) in delivered.iter_mut().enumerate() {
        if physical & (1 << index) == 0 {
            if let Some(pos) = slot.take() {
                return Some((
                    pos,
                    CaptureEvent::Input(Event::Pointer(PointerEvent::Button {
                        time: 0,
                        button: BTN_LEFT + index as u32,
                        state: 0,
                    })),
                ));
            }
        }
    }
    None
}
#[cfg(test)]
mod release_repair_tests {
    use super::*;
    #[test]
    fn dropped_up_is_repaired_without_releasing_held_or_creating_down() {
        let mut delivered = [Some(Position::Left), Some(Position::Left), None, None, None];
        let (_, event) = repair_release(&mut delivered, 2).unwrap();
        assert!(matches!(
            event,
            CaptureEvent::Input(Event::Pointer(PointerEvent::Button {
                button: BTN_LEFT,
                state: 0,
                ..
            }))
        ));
        assert!(repair_release(&mut delivered, 2).is_none());
        assert!(repair_release(&mut delivered, 31).is_none());
    }
}
