use super::*;
use crate::scheduler::{scheduler, Runnable, Shared};
use crate::virtual_dom::{VDiff, VNode};
use cfg_if::cfg_if;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
cfg_if! {
    if #[cfg(feature = "std_web")] {
        use stdweb::web::Element;
    } else if #[cfg(feature = "web_sys")] {
        use web_sys::Element;
    }
}

/// Updates for a `Component` instance. Used by scope sender.
pub(crate) enum ComponentUpdate<COMP: Component> {
    /// Wraps messages for a component.
    Message(COMP::Message),
    /// Wraps batch of messages for a component.
    MessageBatch(Vec<COMP::Message>),
    /// Wraps properties for a component.
    Properties(COMP::Properties),
}

/// A context which allows sending messages to a component.
pub struct Scope<COMP: Component> {
    shared_state: Shared<ComponentState<COMP>>,
}

impl<COMP: Component> fmt::Debug for Scope<COMP> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Scope<_>")
    }
}

impl<COMP: Component> Clone for Scope<COMP> {
    fn clone(&self) -> Self {
        Scope {
            shared_state: self.shared_state.clone(),
        }
    }
}

impl<COMP: Component> Default for Scope<COMP> {
    fn default() -> Self {
        Scope::new()
    }
}

impl<COMP: Component> Scope<COMP> {
    /// visible for testing
    pub fn new() -> Self {
        let shared_state = Rc::new(RefCell::new(ComponentState::Empty));
        Scope { shared_state }
    }

    /// Mounts a component with `props` to the specified `element` in the DOM.
    pub(crate) fn mount_in_place(
        self,
        element: Element,
        ancestor: Option<VNode>,
        node_ref: NodeRef,
        props: COMP::Properties,
    ) -> Scope<COMP> {
        let mut scope = self;
        let ready_state = ReadyState {
            element,
            node_ref,
            scope: scope.clone(),
            props,
            ancestor,
        };
        *scope.shared_state.borrow_mut() = ComponentState::Ready(ready_state);
        scope.create();
        scope.mounted();
        scope
    }

    /// Schedules a task to call the mounted method on a component and optionally re-render
    pub(crate) fn mounted(&mut self) {
        let shared_state = self.shared_state.clone();
        let mounted = MountedComponent { shared_state };
        scheduler().push_mount(Box::new(mounted));
    }

    /// Schedules a task to create and render a component and then mount it to the DOM
    pub(crate) fn create(&mut self) {
        let shared_state = self.shared_state.clone();
        let create = CreateComponent { shared_state };
        scheduler().push_create(Box::new(create));
    }

    /// Schedules a task to send a message or new props to a component
    pub(crate) fn update(&self, update: ComponentUpdate<COMP>) {
        let update = UpdateComponent {
            shared_state: self.shared_state.clone(),
            update,
        };
        scheduler().push(Box::new(update));
    }

    /// Schedules a task to destroy a component
    pub(crate) fn destroy(&mut self) {
        let shared_state = self.shared_state.clone();
        let destroy = DestroyComponent { shared_state };
        scheduler().push(Box::new(destroy));
    }

    /// Send a message to the component
    pub fn send_message(&self, msg: COMP::Message) {
        self.update(ComponentUpdate::Message(msg));
    }

    /// Send a batch of messages to the component
    pub fn send_message_batch(&self, messages: Vec<COMP::Message>) {
        self.update(ComponentUpdate::MessageBatch(messages));
    }

    /// This method creates a `Callback` which will send a message to the linked component's
    /// update method when invoked.
    pub fn callback<F, IN>(&self, function: F) -> Callback<IN>
    where
        F: Fn(IN) -> COMP::Message + 'static,
    {
        let scope = self.clone();
        let closure = move |input| {
            let output = function(input);
            scope.send_message(output);
        };
        closure.into()
    }

    /// This method creates a `Callback` which will send a batch of messages back to the linked
    /// component's update method when called.
    pub fn batch_callback<F, IN>(&self, function: F) -> Callback<IN>
    where
        F: Fn(IN) -> Vec<COMP::Message> + 'static,
    {
        let scope = self.clone();
        let closure = move |input| {
            let messages = function(input);
            scope.send_message_batch(messages);
        };
        closure.into()
    }
}

enum ComponentState<COMP: Component> {
    Empty,
    Ready(ReadyState<COMP>),
    Created(CreatedState<COMP>),
    Processing,
    Destroyed,
}

impl<COMP: Component> fmt::Display for ComponentState<COMP> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ComponentState::Empty => "empty",
            ComponentState::Ready(_) => "ready",
            ComponentState::Created(_) => "created",
            ComponentState::Processing => "processing",
            ComponentState::Destroyed => "destroyed",
        };
        write!(f, "{}", name)
    }
}

struct ReadyState<COMP: Component> {
    element: Element,
    node_ref: NodeRef,
    props: COMP::Properties,
    scope: Scope<COMP>,
    ancestor: Option<VNode>,
}

impl<COMP: Component> ReadyState<COMP> {
    fn create(self) -> CreatedState<COMP> {
        CreatedState {
            component: COMP::create(self.props, self.scope),
            element: self.element,
            last_frame: self.ancestor,
            node_ref: self.node_ref,
        }
    }
}

struct CreatedState<COMP: Component> {
    element: Element,
    component: COMP,
    last_frame: Option<VNode>,
    node_ref: NodeRef,
}

impl<COMP: Component> CreatedState<COMP> {
    /// Called once immediately after the component is created.
    fn mounted(mut self) -> Self {
        if self.component.mounted() {
            self.update()
        } else {
            self
        }
    }

    fn update(mut self) -> Self {
        let mut root = self.component.render();
        if let Some(node) = root.apply(&self.element, None, self.last_frame) {
            self.node_ref.set(Some(node));
        } else if let VNode::VComp(child) = &root {
            // If the root VNode is a VComp, we won't have access to the rendered DOM node
            // because components render asynchronously. In order to bubble up the DOM node
            // from the VComp, we need to link the currently rendering component with its
            // root child component.
            self.node_ref.link(child.node_ref.clone());
        }
        self.last_frame = Some(root);
        self
    }
}

struct MountedComponent<COMP>
where
    COMP: Component,
{
    shared_state: Shared<ComponentState<COMP>>,
}

impl<COMP> Runnable for MountedComponent<COMP>
where
    COMP: Component,
{
    fn run(self: Box<Self>) {
        let current_state = self.shared_state.replace(ComponentState::Processing);
        self.shared_state.replace(match current_state {
            ComponentState::Created(state) => ComponentState::Created(state.mounted()),
            ComponentState::Destroyed => current_state,
            ComponentState::Empty | ComponentState::Processing | ComponentState::Ready(_) => {
                panic!("unexpected component state: {}", current_state);
            }
        });
    }
}

struct CreateComponent<COMP>
where
    COMP: Component,
{
    shared_state: Shared<ComponentState<COMP>>,
}

impl<COMP> Runnable for CreateComponent<COMP>
where
    COMP: Component,
{
    fn run(self: Box<Self>) {
        let current_state = self.shared_state.replace(ComponentState::Processing);
        self.shared_state.replace(match current_state {
            ComponentState::Ready(state) => ComponentState::Created(state.create().update()),
            ComponentState::Created(_) | ComponentState::Destroyed => current_state,
            ComponentState::Empty | ComponentState::Processing => {
                panic!("unexpected component state: {}", current_state);
            }
        });
    }
}

struct DestroyComponent<COMP>
where
    COMP: Component,
{
    shared_state: Shared<ComponentState<COMP>>,
}

impl<COMP> Runnable for DestroyComponent<COMP>
where
    COMP: Component,
{
    fn run(self: Box<Self>) {
        match self.shared_state.replace(ComponentState::Destroyed) {
            ComponentState::Created(mut this) => {
                this.component.destroy();
                if let Some(last_frame) = &mut this.last_frame {
                    last_frame.detach(&this.element);
                }
            }
            ComponentState::Ready(mut this) => {
                if let Some(ancestor) = &mut this.ancestor {
                    ancestor.detach(&this.element);
                }
            }
            ComponentState::Empty | ComponentState::Destroyed => {}
            s @ ComponentState::Processing => panic!("unexpected component state: {}", s),
        };
    }
}

struct UpdateComponent<COMP>
where
    COMP: Component,
{
    shared_state: Shared<ComponentState<COMP>>,
    update: ComponentUpdate<COMP>,
}

impl<COMP> Runnable for UpdateComponent<COMP>
where
    COMP: Component,
{
    fn run(self: Box<Self>) {
        let current_state = self.shared_state.replace(ComponentState::Processing);
        self.shared_state.replace(match current_state {
            ComponentState::Created(mut this) => {
                let should_update = match self.update {
                    ComponentUpdate::Message(message) => this.component.update(message),
                    ComponentUpdate::MessageBatch(messages) => messages
                        .into_iter()
                        .fold(false, |acc, msg| this.component.update(msg) || acc),
                    ComponentUpdate::Properties(props) => this.component.change(props),
                };
                let next_state = if should_update { this.update() } else { this };
                ComponentState::Created(next_state)
            }
            ComponentState::Destroyed => current_state,
            ComponentState::Processing | ComponentState::Ready(_) | ComponentState::Empty => {
                panic!("unexpected component state: {}", current_state);
            }
        });
    }
}

struct Hidden;

pub(crate) struct HiddenScope {
    type_id: TypeId,
    scope: *mut Hidden,
}

impl<COMP: Component> From<Scope<COMP>> for HiddenScope {
    fn from(scope: Scope<COMP>) -> Self {
        HiddenScope {
            type_id: TypeId::of::<COMP>(),
            scope: Box::into_raw(Box::new(scope)) as *mut Hidden,
        }
    }
}

impl<COMP: Component> Into<Scope<COMP>> for HiddenScope {
    fn into(self: HiddenScope) -> Scope<COMP> {
        if self.type_id != TypeId::of::<COMP>() {
            panic!("encountered unespected component type");
        }

        unsafe {
            let raw: *mut Scope<COMP> = self.scope as *mut Scope<COMP>;
            *Box::from_raw(raw)
        }
    }
}
