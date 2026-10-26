//! A console in a test window: the runtime is replaced by a recorder of the
//! commands the console sends, and events go in through the console's own
//! pump, so tests drive the console as the runtime and the operator would.

use std::rc::Rc;

use gpui::{Context, Entity, TestAppContext, VisualTestContext, Window};
use time::UtcOffset;
use tokio::sync::mpsc::UnboundedReceiver;
use voltrus_scada_native::api::DataSource;
use voltrus_scada_native::runtime::{CallKey, Command, Event, Handle, Link, Reply};
use vt_ui::theme::{self, ThemeCatalog};

use super::{Console, Launch, Phase};

/// The server a test session signs in to.
pub(crate) const SERVER: &str = "http://scada:3000";

pub(crate) struct Harness {
    pub cx: VisualTestContext,
    pub console: Entity<Console>,
    commands: UnboundedReceiver<Command>,
    events: async_channel::Sender<Event>,
}

impl Harness {
    /// A console at the sign-in card.
    pub fn new(cx: &mut TestAppContext) -> Self {
        let catalog = Rc::new(ThemeCatalog::bundled());
        let initial = catalog.resolve(theme::DEFAULT_DARK, true);
        // The console binds its keymap itself (`shortcuts::rebind`).
        cx.update(|cx| vt_ui::init(initial, cx));
        let (runtime, commands) = Handle::recorder();
        let (events, receiver) = async_channel::unbounded();
        let launch = Launch {
            sign_in: None,
            page: None,
            tag: None,
            range: None,
            hud: false,
            overlay: None,
            stress: None,
            screen: None,
            edit: false,
            section: None,
            utc_offset: UtcOffset::UTC,
            appearance: Some(true),
            keys: None,
        };
        let (console, vcx) = cx.add_window_view(|window, cx| {
            Console::new(runtime, receiver, catalog, launch, window, cx)
        });
        let mut harness = Self {
            cx: vcx.clone(),
            console,
            commands,
            events,
        };
        harness.cx.run_until_parked();
        harness.commands();
        harness
    }

    /// Signed in as `role` ("admin", "demo" or "viewer"), in the console
    /// with every page warmed; the commands sent so far are dropped.
    pub fn sign_in(&mut self, role: &str) {
        self.event(Event::SignedIn {
            username: "op".into(),
            role: role.into(),
            server: SERVER.into(),
            must_change_password: false,
        });
        self.event(Event::Link(Link::Live));
        for _ in 0..64 {
            let warming = self.update(|console, _, _| !console.warm.is_empty());
            if !warming {
                break;
            }
            self.draw();
        }
        assert_eq!(self.update(|console, _, _| console.phase), Phase::Console);
        self.commands();
    }

    /// A minimal tag for [`Event::Tags`].
    pub fn tag(id: i64, key: &str) -> DataSource {
        DataSource {
            id,
            key: key.into(),
            name: key.into(),
            source_type: "modbus".into(),
            max_val: 100.0,
            decimals: 1,
            ..DataSource::default()
        }
    }

    /// An event from the runtime, applied by the console's pump.
    pub fn event(&mut self, event: Event) {
        self.events
            .try_send(event)
            .expect("the console's pump is listening");
        self.cx.run_until_parked();
    }

    /// The answer to a generic REST call.
    pub fn reply(&mut self, key: CallKey, reply: Reply) {
        self.event(Event::Reply { key, reply });
    }

    /// Every command sent since the last look.
    pub fn commands(&mut self) -> Vec<Command> {
        let mut sent = Vec::new();
        while let Ok(command) = self.commands.try_recv() {
            sent.push(command);
        }
        sent
    }

    /// The keys of the generic REST calls sent since the last look (other
    /// commands are dropped).
    pub fn calls(&mut self) -> Vec<CallKey> {
        self.commands()
            .into_iter()
            .filter_map(|command| match command {
                Command::Call { key, .. } => Some(key),
                _ => None,
            })
            .collect()
    }

    /// Keystrokes, space-separated (`"tab enter"`, `"shift-cmd-w"`).
    pub fn keys(&mut self, keystrokes: &str) {
        self.cx.simulate_keystrokes(keystrokes);
    }

    /// Draw the window afresh.
    pub fn draw(&mut self) {
        self.cx.update(|window, _| window.refresh());
        self.cx.run_until_parked();
    }

    /// Run `f` on the console with its window.
    pub fn update<R>(
        &mut self,
        f: impl FnOnce(&mut Console, &mut Window, &mut Context<Console>) -> R,
    ) -> R {
        let console = self.console.clone();
        let result = self
            .cx
            .update(|window, cx| console.update(cx, |console, cx| f(console, window, cx)));
        self.cx.run_until_parked();
        result
    }
}
