// SPDX-License-Identifier: Apache-2.0
//! Stable state for the bounded visible portion of a virtual list.
//!
//! Iced 0.14's keyed column only adjusts child positions when their count changes.
//! A virtual viewport often replaces rows at an unchanged count, so reconcile
//! by actual identities before delegating normal layout/event handling to Column.
use crate::Msg;
use flow_gui::model::reconcile_visible_rows;
use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{tree, Operation, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    widget::Column,
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
};

pub fn column<'a>(rows: Vec<(String, Element<'a, Msg>)>) -> Element<'a, Msg> {
    let (keys, children): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    Element::new(StableRows {
        keys,
        content: Column::with_children(children).into(),
    })
}

struct StableRows<'a> {
    keys: Vec<String>,
    content: Element<'a, Msg>,
}
struct State {
    keys: Vec<String>,
}

impl Widget<Msg, Theme, Renderer> for StableRows<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            keys: self.keys.clone(),
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.content.as_widget())]
    }
    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        let content = &mut tree.children[0];
        if state.keys != self.keys {
            let old_keys = std::mem::take(&mut state.keys);
            let old_children = std::mem::take(&mut content.children);
            // Allocate fresh widget state only when membership/order changes;
            // ordinary progress updates retain the existing tree directly.
            let mut fresh = self
                .content
                .as_widget()
                .children()
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>();
            content.children =
                reconcile_visible_rows(old_keys, old_children, &self.keys, |index| {
                    fresh[index].take().expect("one state per unique row")
                });
            state.keys = self.keys.clone();
        }
        self.content.as_widget().diff(content);
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation)
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Msg>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Msg, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
