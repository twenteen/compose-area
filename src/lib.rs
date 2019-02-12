//! Note: This library is not thread safe!

use virtual_dom_rs;
use web_sys;

mod keys;
mod state;
mod utils;

use std::mem;

use cfg_if::cfg_if;
use virtual_dom_rs::{VirtualNode, VElement};
use wasm_bindgen::prelude::*;
use web_sys::{Element, Node, NodeList, Range};

use crate::keys::Key;
use crate::state::{State, Direction};

cfg_if! {
    // When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
    // allocator.
    if #[cfg(feature = "wee_alloc")] {
        extern crate wee_alloc;
        #[global_allocator]
        static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
    }
}

pub struct Context {
    pub state: State,
    pub wrapper_id: String,
}

/// Wrap the list of virtual nodes in a content editable wrapper element.
fn wrap(virtual_nodes: Vec<VirtualNode>, wrapper_id: &str) -> VirtualNode {
    let mut wrapper = VElement::new("div");
    wrapper.props.insert("id".into(), wrapper_id.to_string());
    wrapper.props.insert("class".into(), "cawrapper initialized".into());
    wrapper.props.insert("contenteditable".into(), "true".into());
    wrapper.children = virtual_nodes;
    wrapper.into()
}

/// Initialize a new compose area wrapper with the specified `id`.
#[wasm_bindgen]
pub fn bind_to(id: &str) -> *mut Context {
    utils::set_panic_hook();

    web_sys::console::log_1(&format!("Bind to #{}", id).into());

    let window = web_sys::window().expect("No global `window` exists");
    let document = window.document().expect("Should have a document on window");
    let wrapper: Element = document.get_element_by_id(id).expect("Did not find element");

    // Initialize the wrapper element with the initial empty DOM.
    // This prevents the case where the wrapper element is not initialized as
    // it should be, which can lead to funny errors when patching.
    let state = State::new();
    let initial_vdom: VirtualNode = wrap(state.to_virtual_nodes(), id);
    let initial_dom: Node = initial_vdom.create_dom_node().node;
    wrapper.replace_with_with_node_1(&initial_dom)
        .expect("Could not initialize wrapper");

    web_sys::console::log_1(&format!("Initialized #{}", id).into());

    let ctx = Box::new(Context {
        state,
        wrapper_id: id.to_owned(),
    });
    Box::into_raw(ctx)
}

pub fn set_inner_html(id: &str, html: &str) {
    let window = web_sys::window().expect("no global `window` exists");
    let document = window.document().expect("should have a document on window");
    let wrapper = document.get_element_by_id(id).expect("did not find element");
    wrapper.set_inner_html(html);
}

/// A position relative to a node.
enum Position<'a> {
    After(&'a Node),
    Offset(&'a Node, u32),
}

fn add_range_at(pos: Position) {
    web_sys::console::debug_1(&"add_range_at".into());

    let window = web_sys::window().expect("no global `window` exists");
    let document = window.document().expect("should have a document on window");

    let range: Range = document.create_range().expect("Could not create range");
    match pos {
        Position::After(node) => {
            range.set_start_after(node).expect("Could not set range start after");
            range.set_end_after(node).expect("Could not set range end after");
        }
        Position::Offset(node, 0) => {
            range.set_start_before(node).expect("Could not set range start before");
            range.set_end_before(node).expect("Could not set range end before");
        }
        Position::Offset(node, offset) => {
            range.set_start(node, offset).expect("Could not set range start");
            range.set_end(node, offset).expect("Could not set range end");
        }
    }

    if let Some(sel) = window.get_selection().expect("Could not get selection from window") {
        sel.remove_all_ranges().expect("Could not remove ranges");
        sel.add_range(&range).expect("Could not add range");
    } else {
        // TODO warn
    }
}

fn browser_set_caret_position(wrapper: &Element, state: &State) {
    web_sys::console::debug_1(&"browser_set_caret_position".into());

    let nodes: NodeList = wrapper.child_nodes();
    let node_count = nodes.length();
    assert_eq!(node_count, state.node_count() as u32 + 1);

    if let Some(pos) = state.find_start_node(Direction::After) {
        match nodes.get(pos.index as u32) {
            Some(ref node) => add_range_at(Position::Offset(&node, pos.offset as u32)),
            None => { /* TODO */ }
        }
    } else {
        // We're at the end of the node list. Use the latest node.
        match nodes.get(node_count - 1) {
            Some(ref node) => add_range_at(Position::After(&node)),
            None => { /* TODO */ },
        }
    }
}

/// Return whether the default event handler should be prevented from running.
#[wasm_bindgen]
pub fn process_key(ctx: *mut Context, key_val: &str) -> bool {
    // Validate and parse key value
    if key_val.len() == 0 {
        web_sys::console::warn_1(&"process_key: No key value provided".into());
        return false;
    }
    let key = match Key::from_str(key_val) {
        Some(key) => key,
        None => return false,
    };

    // Dereference context
    let mut context = unsafe { Box::from_raw(ctx) };

    // Get access to wrapper element
    let window = web_sys::window().expect("no global `window` exists");
    let document = window.document().expect("should have a document on window");
    let wrapper = document.get_element_by_id(&context.wrapper_id).expect("did not find element");

    // Get old virtual DOM
    let old_vdom = wrap(context.state.to_virtual_nodes(), &context.wrapper_id);

    // Handle input
    context.state.handle_key(key);

    // Get new virtual DOM
    let new_vdom = wrap(context.state.to_virtual_nodes(), &context.wrapper_id);

    // Do the DOM diffing
    let patches = virtual_dom_rs::diff(&old_vdom, &new_vdom);

    web_sys::console::log_1(&format!("RS: Old vdom: {:?}", &old_vdom).into());
    web_sys::console::log_1(&format!("RS: New vdom: {:?}", &new_vdom).into());
    web_sys::console::log_1(&format!("RS: Patches {:?}", &patches).into());

    // Patch the current DOM
    virtual_dom_rs::patch(wrapper.clone(), &patches);

    // Update the caret position in the browser
    browser_set_caret_position(&wrapper, &context.state);

    // Forget about the context box to prevent it from being freed
    mem::forget(context);

    // We handled the event, so prevent the default event from being handled.
    true
}

/// Set the start and end of the caret position (relative to the HTML).
#[wasm_bindgen]
pub fn update_caret_position(ctx: *mut Context, start: usize, end: usize) {
    // Dereference context
    let mut context = unsafe { Box::from_raw(ctx) };

    // Update state
    if end < start {
        return;
    }
    context.state.set_caret_position(start, end);

    // Forget about the context box to prevent it from being freed
    mem::forget(context);
}

/// Dipose all state related to the specified context.
///
/// After calling this function, the context may not be used anymore.
#[wasm_bindgen]
pub fn dispose(ctx: *mut Context) {
    // Dereference context and drop
    unsafe { Box::from_raw(ctx); }
}
