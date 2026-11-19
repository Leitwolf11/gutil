#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use super::{copy_stream, ErrorMode, Output, Sink};
use crate::{config::Settings, presentation::Presentation};

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    closed: bool,
}

struct DeferredFailure {
    state: Rc<RefCell<State>>,
    write_error: Option<io::ErrorKind>,
    close_error: Option<io::ErrorKind>,
}

impl Sink for DeferredFailure {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Some(kind) = self.write_error {
            return Err(io::Error::new(kind, "injected write failure"));
        }
        self.state.borrow_mut().bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn finish(self) -> io::Result<()> {
        self.state.borrow_mut().closed = true;
        self.close_error.map_or(Ok(()), |kind| {
            Err(io::Error::new(kind, "delayed close failure"))
        })
    }
}

fn output(
    name: &str,
    state: &Rc<RefCell<State>>,
    close_error: Option<io::ErrorKind>,
) -> Output<DeferredFailure> {
    Output {
        name: name.to_owned(),
        writer: DeferredFailure {
            state: state.clone(),
            write_error: None,
            close_error,
        },
    }
}

#[test]
fn successful_writes_do_not_hide_close_time_failures() {
    let payload = vec![73; 300_003];
    for mode in [
        ErrorMode::Default,
        ErrorMode::Warn,
        ErrorMode::WarnNoPipe,
        ErrorMode::Exit,
        ErrorMode::ExitNoPipe,
    ] {
        let first = Rc::new(RefCell::new(State::default()));
        let second = Rc::new(RefCell::new(State::default()));
        let mut errors = Vec::new();
        let status = copy_stream(
            &mut payload.as_slice(),
            vec![
                output("delayed", &first, Some(io::ErrorKind::StorageFull)),
                output("survivor", &second, None),
            ],
            mode,
            Presentation::new(&Settings::default()),
            &mut errors,
            false,
            None,
        )
        .unwrap();
        assert_eq!(status, 1);
        assert_eq!(first.borrow().bytes, payload);
        assert_eq!(second.borrow().bytes, payload);
        assert!(first.borrow().closed);
        assert_eq!(second.borrow().closed, !mode.exits());
        assert!(String::from_utf8_lossy(&errors).contains("delayed: delayed close failure"));
    }
}

#[test]
fn close_time_broken_pipes_follow_the_selected_policy() {
    for (mode, expected) in [
        (ErrorMode::Warn, 1),
        (ErrorMode::WarnNoPipe, 0),
        (ErrorMode::Exit, 1),
        (ErrorMode::ExitNoPipe, 0),
    ] {
        let state = Rc::new(RefCell::new(State::default()));
        let mut errors = Vec::new();
        let status = copy_stream(
            &mut b"bytes".as_slice(),
            vec![output("pipe", &state, Some(io::ErrorKind::BrokenPipe))],
            mode,
            Presentation::new(&Settings::default()),
            &mut errors,
            false,
            None,
        )
        .unwrap();
        assert_eq!(status, expected);
        assert_eq!(errors.is_empty(), expected == 0);
        assert!(state.borrow().closed);
    }
}

#[test]
fn ignored_write_failures_do_not_hide_non_pipe_close_errors() {
    let failed = Rc::new(RefCell::new(State::default()));
    let survivor = Rc::new(RefCell::new(State::default()));
    let mut broken = output("broken", &failed, Some(io::ErrorKind::StorageFull));
    broken.writer.write_error = Some(io::ErrorKind::BrokenPipe);
    let mut errors = Vec::new();
    let status = copy_stream(
        &mut b"complete stream".as_slice(),
        vec![broken, output("survivor", &survivor, None)],
        ErrorMode::WarnNoPipe,
        Presentation::new(&Settings::default()),
        &mut errors,
        false,
        None,
    )
    .unwrap();
    assert_eq!(status, 1);
    assert!(failed.borrow().closed);
    assert_eq!(survivor.borrow().bytes, b"complete stream");
    assert!(survivor.borrow().closed);
    assert!(String::from_utf8_lossy(&errors).contains("delayed close failure"));
}
