// SPDX-License-Identifier: Apache-2.0
//! A modal background is drawn but does not receive pointer, keyboard or focus operations.
use crate::Msg;
use iced::{
    advanced::{
        layout, mouse, renderer,
        widget::{tree, Tree},
        Layout, Widget,
    },
    Element, Length, Rectangle, Renderer, Size, Theme,
};

pub fn inert(content: Element<'_, Msg>) -> Element<'_, Msg> {
    Element::new(Inert(content))
}
struct Inert<'a>(Element<'a, Msg>);
impl Widget<Msg, Theme, Renderer> for Inert<'_> {
    fn size(&self) -> Size<Length> {
        self.0.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.0.as_widget().size_hint()
    }
    fn tag(&self) -> tree::Tag {
        self.0.as_widget().tag()
    }
    fn state(&self) -> tree::State {
        self.0.as_widget().state()
    }
    fn children(&self) -> Vec<Tree> {
        self.0.as_widget().children()
    }
    fn diff(&self, tree: &mut Tree) {
        self.0.as_widget().diff(tree)
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.0.as_widget_mut().layout(tree, renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.0.as_widget().draw(
            tree,
            renderer,
            theme,
            style,
            layout,
            mouse::Cursor::Unavailable,
            viewport,
        )
    }
}
