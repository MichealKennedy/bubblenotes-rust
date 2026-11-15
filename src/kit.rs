//! The editor setup, built once for the app: the schema, plugins and named
//! commands every document shares. Each open document is an [`Editor`]: its
//! own session (state, undo history) over the shared [`Kit`].
//!
//! Building the setup once matters: a document's node types belong to the
//! schema that made them, and an editor can't edit a document of another
//! schema instance (its edits are silently refused).

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use nsh_base::rich_editor::{
    commands::{BoxCommand, CommandRegistry},
    extensions::{
        Extension, ExtensionManager,
        kit::{starter_kit, table_kit},
        markdown::{MarkdownOptions, markdown},
    },
    model::{Attrs, Node, Schema},
    state::{EditorState, EditorStateConfig, Plugin},
    view::{EditorSession, NodeViews},
};
use serde_json::Value;

use crate::{slash, toggle};

// ----- What the editor tells the app ----------------------------------------

/// What reaches the app from the editor outside its view's events.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppEvent {
    /// File shortcuts pressed while the editor has the focus (a focused
    /// `RichEditor` binds every Ctrl/⌘ keystroke itself, so window shortcuts
    /// don't fire then; its keymap gets them instead). There's no Save As
    /// key: Ctrl/⌘+Shift+S is strikethrough, as in Notion and Tiptap.
    New,
    Open,
    Save,
    /// Ctrl/⌘+K: edit the link at the selection.
    Link,
    /// The editor's state changed, also when only a plugin's state did (the
    /// slash menu's captured keys), which the view's events don't report.
    Changed,
}

/// Carries [`AppEvent`]s from the editor (whose keymap commands and
/// subscribers are `Send + Sync`, and may run off the engine thread) to a
/// task on the app's side.
#[derive(Clone, Default)]
pub struct Relay(Arc<Mutex<(Vec<AppEvent>, Option<Waker>)>>);

impl Relay {
    pub fn push(&self, event: AppEvent) {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Many changes in a row are one wake-up.
        if event != AppEvent::Changed || inner.0.last() != Some(&AppEvent::Changed) {
            inner.0.push(event);
        }
        if let Some(waker) = inner.1.take() {
            waker.wake();
        }
    }

    /// The events since the last call, once there are any.
    pub async fn next(&self) -> Vec<AppEvent> {
        std::future::poll_fn(|cx| {
            let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if inner.0.is_empty() {
                inner.1 = Some(cx.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(std::mem::take(&mut inner.0))
            }
        })
        .await
    }

    /// An editor extension binding the file shortcuts to this relay.
    fn shortcuts(&self) -> Extension {
        let relay = self.clone();
        Extension::new("appShortcuts").add_keyboard_shortcuts(move |_| {
            [
                ("Mod-n", AppEvent::New),
                ("Mod-o", AppEvent::Open),
                ("Mod-s", AppEvent::Save),
                ("Mod-k", AppEvent::Link),
            ]
            .into_iter()
            .map(|(keys, event)| {
                let relay = relay.clone();
                let command: BoxCommand = Arc::new(move |_, dispatch| {
                    // Only a real key press, not a "could it run?" check.
                    if dispatch.is_some() {
                        relay.push(event);
                    }
                    true
                });
                (keys.to_string(), command)
            })
            .collect()
        })
    }
}

// ----- The shared setup -----------------------------------------------------

/// The schema, plugins and commands, built once for every document.
pub struct Kit {
    pub schema: Schema,
    pub commands: Arc<CommandRegistry>,
    plugins: Vec<Plugin>,
    pub relay: Relay,
}

impl Kit {
    /// Tiptap's starter kit (paragraphs, headings, lists, task lists,
    /// quotes, code, links, images, bold/italic/strike/underline/highlight/
    /// sub/sup, text color, alignment, history, typing shortcuts), tables,
    /// toggles, the slash menu, and the app's file shortcuts.
    pub fn new(relay: Relay) -> Rc<Self> {
        let manager = ExtensionManager::with_core(vec![
            relay.shortcuts(),
            starter_kit(),
            table_kit(),
            toggle::extension(),
        ])
        .expect("the editor's extensions resolve");
        let schema = manager.schema().clone();
        // The slash menu comes first: while it's open, its Enter and arrows
        // come before the keymap's (which would split the paragraph).
        let mut plugins = vec![slash::plugin()];
        plugins.extend(manager.plugins());
        // Markdown pasted as plain text is read as Markdown, and copies put
        // Markdown on the clipboard (the starter kit has the typing rules).
        let mut options = MarkdownOptions::default();
        options.block_rules = false;
        options.inline_rules = false;
        plugins.extend(markdown(options));
        Rc::new(Self { schema, commands: manager.commands().clone(), plugins, relay })
    }

    /// An editor over `doc` (a document of this kit's schema), or an empty
    /// one.
    pub fn editor(self: &Rc<Self>, doc: Option<Node>) -> Editor {
        let mut config = EditorStateConfig::new(self.schema.clone()).plugins(self.plugins.clone());
        if let Some(doc) = doc {
            config = config.doc(doc);
        }
        let state = EditorState::create(config).expect("a document of the kit's schema");
        let session = EditorSession::new(state, NodeViews::new());
        session.set_node_views(toggle::node_views());
        let relay = self.relay.clone();
        session.subscribe(Arc::new(move |_| relay.push(AppEvent::Changed)));
        Editor { session, kit: self.clone() }
    }
}

// ----- One open document ------------------------------------------------------

/// One open document: its live session, over the shared kit.
#[derive(Clone)]
pub struct Editor {
    pub session: EditorSession,
    pub kit: Rc<Kit>,
}

/// Two editors are the same when they share a session.
impl PartialEq for Editor {
    fn eq(&self, other: &Self) -> bool {
        self.session == other.session
    }
}

impl Editor {
    pub fn doc(&self) -> Node {
        self.session.state().doc().clone()
    }

    pub fn state(&self) -> EditorState {
        self.session.state()
    }

    fn command(&self, name: &str, args: &Value) -> Option<BoxCommand> {
        self.kit.commands.make(name, args, &self.kit.schema).ok()
    }

    /// Runs the named command (Tiptap's names: `toggleBold`,
    /// `toggleHeading` with `{ "level": 2 }`, ...) on the selection, then
    /// gives the focus back to the text. Whether it applied.
    pub fn run(&self, name: &str, args: Value) -> bool {
        let applied = self.command(name, &args).is_some_and(|c| self.session.run(&c));
        self.session.focus();
        applied
    }

    /// Runs a command made here (not by name).
    pub fn run_command(&self, command: &BoxCommand) -> bool {
        let applied = self.session.run(command);
        self.session.focus();
        applied
    }

    /// Whether the named command could run now.
    pub fn can(&self, name: &str, args: &Value) -> bool {
        self.command(name, args).is_some_and(|c| self.session.can(&c))
    }

    /// Whether the selection is in the node, or has the mark, `name` (with
    /// `attrs`, when given).
    pub fn is_active(&self, name: &str, attrs: Option<Value>) -> bool {
        let attrs = attrs.and_then(|v| serde_json::from_value::<Attrs>(v).ok());
        self.session.is_active(Some(name), attrs)
    }

    /// The attributes of the mark or node `name` at the selection.
    pub fn attributes(&self, name: &str) -> Attrs {
        self.session.get_attributes(name)
    }
}
