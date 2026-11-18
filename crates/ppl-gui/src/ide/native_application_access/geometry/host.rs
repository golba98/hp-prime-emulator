//! Resolve qualified Geometry reads from a read-only worker snapshot.
use super::{ReadSnapshot, read_only};
use crate::ide::native_geometry::Book;
use ppl_graphics::Grob;
use ppl_vm::{
    ErrorKind, HeadlessHost, Host, PortableValue, RuntimeError, SessionSnapshot,
    host::{AppVariableRequest, Dialog, DialogResult, MouseState, Touch},
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

pub(crate) fn read_only_host(
    book: ReadSnapshot,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    timeout_message: &'static str,
) -> Box<dyn Host> {
    let headless = HeadlessHost {
        interrupt: Some(cancel.clone()),
        ..Default::default()
    };
    match book {
        Some(book) => Box::new(GeometryHost {
            headless,
            book,
            cancel,
            deadline,
            timeout_message,
        }),
        None => Box::new(headless),
    }
}

struct GeometryHost {
    headless: HeadlessHost,
    book: Arc<Book>,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    timeout_message: &'static str,
}

impl Host for GeometryHost {
    fn app_variable_with_context(
        &mut self,
        request: AppVariableRequest,
        context: SessionSnapshot,
    ) -> Result<PortableValue, RuntimeError> {
        if request.app != "Geometry" {
            return self.headless.app_variable_with_context(request, context);
        }
        if Instant::now() >= self.deadline {
            return Err(RuntimeError::new(
                ErrorKind::InvalidInput,
                self.timeout_message,
            ));
        }
        read_only(&self.book, &request, &context, self.cancel.clone())
    }

    fn mouse_state(&mut self) -> Result<MouseState, RuntimeError> {
        self.headless.mouse_state()
    }

    fn print(&mut self, text: &str) {
        self.headless.print(text);
    }

    fn present(&mut self, screen: &Grob, terminal: Option<&[String]>) {
        self.headless.present(screen, terminal);
    }

    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.headless.poll_key()
    }

    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError> {
        self.headless.is_key_down(key)
    }

    fn wait_key(&mut self, timeout: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.headless.wait_key(timeout)
    }

    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError> {
        self.headless.sleep(seconds)
    }

    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.headless.touch()
    }

    fn dialog(&mut self, dialog: &Dialog, screen: &Grob) -> Result<DialogResult, RuntimeError> {
        self.headless.dialog(dialog, screen)
    }

    fn now_ms(&mut self) -> f64 {
        self.headless.now_ms()
    }

    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.headless.local_time()
    }

    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        self.headless.interrupt_flag()
    }
}
