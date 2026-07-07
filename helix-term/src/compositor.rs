// Each component declares its own size constraints and gets fitted based on its parent.
// Q: how does this work with popups?
// cursive does compositor.screen_mut().add_layer_at(pos::absolute(x, y), <component>)
use helix_core::Position;
use helix_view::graphics::{CursorKind, Rect};

use tui::buffer::{Buffer as Surface, Cell};

pub type Callback = Box<dyn FnOnce(&mut Compositor, &mut Context)>;
pub type SyncCallback = Box<dyn FnOnce(&mut Compositor, &mut Context) + Sync>;

// Cursive-inspired
pub enum EventResult {
    Ignored(Option<Callback>),
    Consumed(Option<Callback>),
}

use crate::job::Jobs;
use crate::ui::picker;
use helix_view::Editor;

pub use helix_view::input::Event;

pub struct Context<'a> {
    pub editor: &'a mut Editor,
    pub scroll: Option<usize>,
    pub jobs: &'a mut Jobs,
}

impl Context<'_> {
    /// Waits on all pending jobs, and then tries to flush all pending write
    /// operations for all documents.
    pub fn block_try_flush_writes(&mut self) -> anyhow::Result<()> {
        tokio::task::block_in_place(|| helix_lsp::block_on(self.jobs.finish(self.editor, None)))?;
        tokio::task::block_in_place(|| helix_lsp::block_on(self.editor.flush_writes()))?;
        Ok(())
    }
}

pub trait Component: Any + AnyComponent {
    /// Process input events, return true if handled.
    fn handle_event(&mut self, _event: &Event, _ctx: &mut Context) -> EventResult {
        EventResult::Ignored(None)
    }
    // , args: ()

    /// Should redraw? Useful for saving redraw cycles if we know component didn't change.
    fn should_update(&self) -> bool {
        true
    }

    /// Region owned by this component when rendered into `area`.
    ///
    /// Components that return `None` are treated as transparent or unbounded and
    /// are rendered directly unless a valid cache already exists.
    fn render_region(&self, _area: Rect) -> Option<Rect> {
        None
    }

    /// Render the component onto the provided surface.
    fn render(&mut self, area: Rect, frame: &mut Surface, ctx: &mut Context);

    /// Get cursor position and cursor kind.
    fn cursor(&self, _area: Rect, _ctx: &Editor) -> (Option<Position>, CursorKind) {
        (None, CursorKind::Hidden)
    }

    /// May be used by the parent component to compute the child area.
    /// viewport is the maximum allowed area, and the child should stay within those bounds.
    ///
    /// The returned size might be larger than the viewport if the child is too big to fit.
    /// In this case the parent can use the values to calculate scroll.
    fn required_size(&mut self, _viewport: (u16, u16)) -> Option<(u16, u16)> {
        None
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn id(&self) -> Option<&'static str> {
        None
    }
}

pub struct Compositor {
    layers: Vec<Box<dyn Component>>,
    layer_caches: Vec<LayerCache>,
    area: Rect,

    pub(crate) last_picker: Option<Box<dyn Component>>,
    pub(crate) full_redraw: bool,
}

#[derive(Default)]
struct LayerCache {
    area: Option<Rect>,
    region: Option<Rect>,
    cells: Vec<CachedCell>,
}

struct CachedCell {
    index: usize,
    cell: Cell,
}

impl LayerCache {
    fn is_valid_for(&self, area: Rect, region: Option<Rect>) -> bool {
        self.area == Some(area) && self.region == region
    }

    fn invalidate(&mut self) {
        self.area = None;
        self.region = None;
        self.cells.clear();
    }

    fn capture(&mut self, area: Rect, region: Rect, surface: &Surface) {
        self.area = Some(area);
        self.region = Some(region);
        self.cells.clear();

        for y in region.top()..region.bottom() {
            for x in region.left()..region.right() {
                let index = surface.index_of(x, y);
                self.cells.push(CachedCell {
                    index,
                    cell: surface.content[index].clone(),
                });
            }
        }
    }

    fn replay(&self, surface: &mut Surface) {
        for CachedCell { index, cell } in &self.cells {
            surface.content[*index] = cell.clone();
        }
    }
}

impl Compositor {
    pub fn new(area: Rect) -> Self {
        Self {
            layers: Vec::new(),
            layer_caches: Vec::new(),
            area,
            last_picker: None,
            full_redraw: false,
        }
    }

    pub fn size(&self) -> Rect {
        self.area
    }

    pub fn resize(&mut self, area: Rect) {
        self.area = area;
        self.invalidate_render_cache();
    }

    /// Add a layer to be rendered in front of all existing layers.
    pub fn push(&mut self, mut layer: Box<dyn Component>) {
        // immediately clear last_picker field to avoid excessive memory
        // consumption for picker with many items
        if layer.id() == Some(picker::ID) {
            self.last_picker = None;
        }
        let size = self.size();
        // trigger required_size on init
        layer.required_size((size.width, size.height));
        self.layers.push(layer);
        self.layer_caches.push(LayerCache::default());
    }

    /// Replace a component that has the given `id` with the new layer and if
    /// no component is found, push the layer normally.
    pub fn replace_or_push<T: Component>(&mut self, id: &'static str, layer: T) {
        let mut layer = Some(Box::new(layer) as Box<dyn Component>);
        if let Some(idx) = self
            .layers
            .iter()
            .position(|component| component.id() == Some(id))
        {
            self.layers[idx] = layer.take().unwrap();
            self.layer_caches[idx].invalidate();
        } else {
            self.push(layer.take().unwrap())
        }
    }

    pub fn pop(&mut self) -> Option<Box<dyn Component>> {
        self.layer_caches.pop();
        self.layers.pop()
    }

    pub fn remove(&mut self, id: &'static str) -> Option<Box<dyn Component>> {
        let idx = self
            .layers
            .iter()
            .position(|layer| layer.id() == Some(id))?;
        self.layer_caches.remove(idx);
        Some(self.layers.remove(idx))
    }

    pub fn remove_type<T: 'static>(&mut self) {
        let type_name = std::any::type_name::<T>();
        let mut idx = 0;
        while idx < self.layers.len() {
            if self.layers[idx].type_name() == type_name {
                self.layers.remove(idx);
                self.layer_caches.remove(idx);
            } else {
                idx += 1;
            }
        }
    }
    pub fn handle_event(&mut self, event: &Event, cx: &mut Context) -> bool {
        // If it is a key event, a macro is being recorded, and a macro isn't being replayed,
        // push the key event to the recording.
        if let (Event::Key(key), Some((_, keys))) = (event, &mut cx.editor.macro_recording) {
            if cx.editor.macro_replaying.is_empty() {
                keys.push(*key);
            }
        }

        let mut callbacks = Vec::new();
        let mut consumed = false;

        // propagate events through the layers until we either find a layer that consumes it or we
        // run out of layers (event bubbling), starting at the front layer and then moving to the
        // background.
        for layer in self.layers.iter_mut().rev() {
            match layer.handle_event(event, cx) {
                EventResult::Consumed(Some(callback)) => {
                    callbacks.push(callback);
                    consumed = true;
                    break;
                }
                EventResult::Consumed(None) => {
                    consumed = true;
                    break;
                }
                EventResult::Ignored(Some(callback)) => {
                    callbacks.push(callback);
                }
                EventResult::Ignored(None) => {}
            };
        }

        for callback in callbacks {
            callback(self, cx)
        }

        consumed
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        if self.area != area {
            self.area = area;
            self.invalidate_render_cache();
        }

        for (layer, cache) in self.layers.iter_mut().zip(self.layer_caches.iter_mut()) {
            let should_update = layer.should_update();
            let render_region = layer.render_region(area);
            if !should_update && cache.is_valid_for(area, render_region) {
                cache.replay(surface);
            } else if let Some(region) = render_region {
                layer.render(area, surface, cx);
                cache.capture(area, region, surface);
            } else {
                cache.invalidate();
                layer.render(area, surface, cx);
            }
        }
    }

    pub fn cursor(&self, area: Rect, editor: &Editor) -> (Option<Position>, CursorKind) {
        for layer in self.layers.iter().rev() {
            if let (Some(pos), kind) = layer.cursor(area, editor) {
                return (Some(pos), kind);
            }
        }
        (None, CursorKind::Hidden)
    }

    pub fn has_component(&self, type_name: &str) -> bool {
        self.layers
            .iter()
            .any(|component| component.type_name() == type_name)
    }

    pub fn find<T: 'static>(&mut self) -> Option<&mut T> {
        let type_name = std::any::type_name::<T>();
        self.layers
            .iter_mut()
            .find(|component| component.type_name() == type_name)
            .and_then(|component| component.as_any_mut().downcast_mut())
    }

    pub fn find_id<T: 'static>(&mut self, id: &'static str) -> Option<&mut T> {
        self.layers
            .iter_mut()
            .find(|component| component.id() == Some(id))
            .and_then(|component| component.as_any_mut().downcast_mut())
    }

    pub fn need_full_redraw(&mut self) {
        self.full_redraw = true;
        self.invalidate_render_cache();
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    fn invalidate_render_cache(&mut self) {
        for cache in &mut self.layer_caches {
            cache.invalidate();
        }
    }
}

// View casting, taken straight from Cursive

use std::any::Any;

/// A view that can be downcasted to its concrete type.
///
/// This trait is automatically implemented for any `T: Component`.
pub trait AnyComponent {
    /// Downcast self to a `Any`.
    fn as_any(&self) -> &dyn Any;

    /// Downcast self to a mutable `Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Returns a boxed any from a boxed self.
    ///
    /// Can be used before `Box::downcast()`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helix_term::{ui::Text, compositor::Component};
    /// let boxed: Box<dyn Component> = Box::new(Text::new("text".to_string()));
    /// let text: Box<Text> = boxed.as_boxed_any().downcast().unwrap();
    /// ```
    fn as_boxed_any(self: Box<Self>) -> Box<dyn Any>;
}

impl<T: Component> AnyComponent for T {
    /// Downcast self to a `Any`.
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Downcast self to a mutable `Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_boxed_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_symbol(surface: &Surface, x: u16, y: u16) -> &str {
        &surface[(x, y)].symbol
    }

    #[test]
    fn layer_cache_replays_cached_region() {
        let area = Rect::new(0, 0, 3, 1);
        let mut after = Surface::empty(area);
        after[(0, 0)].set_symbol("a");
        after[(1, 0)].set_symbol("x");
        after[(2, 0)].set_symbol("c");

        let mut cache = LayerCache::default();
        cache.capture(area, Rect::new(1, 0, 2, 1), &after);

        let mut next_frame = Surface::empty(area);
        next_frame[(0, 0)].set_symbol("1");
        next_frame[(1, 0)].set_symbol("2");
        next_frame[(2, 0)].set_symbol("3");

        cache.replay(&mut next_frame);

        assert_eq!(cell_symbol(&next_frame, 0, 0), "1");
        assert_eq!(cell_symbol(&next_frame, 1, 0), "x");
        assert_eq!(cell_symbol(&next_frame, 2, 0), "c");
        assert_eq!(cache.region, Some(Rect::new(1, 0, 2, 1)));
    }

    #[test]
    fn layer_cache_invalidate_clears_cached_region() {
        let area = Rect::new(0, 0, 1, 1);
        let mut after = Surface::empty(area);
        after[(0, 0)].set_symbol("x");

        let mut cache = LayerCache::default();
        cache.capture(area, area, &after);
        assert!(cache.is_valid_for(area, Some(area)));

        cache.invalidate();

        assert!(!cache.is_valid_for(area, Some(area)));
        assert_eq!(cache.region, None);
        assert!(cache.cells.is_empty());
    }
}

impl dyn AnyComponent {
    /// Attempts to downcast `self` to a concrete type.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref()
    }

    /// Attempts to downcast `self` to a concrete type.
    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut()
    }

    /// Attempts to downcast `Box<Self>` to a concrete type.
    pub fn downcast<T: Any>(self: Box<Self>) -> Result<Box<T>, Box<Self>> {
        // Do the check here + unwrap, so the error
        // value is `Self` and not `dyn Any`.
        if self.as_any().is::<T>() {
            Ok(self.as_boxed_any().downcast().unwrap())
        } else {
            Err(self)
        }
    }

    /// Checks if this view is of type `T`.
    pub fn is<T: Any>(&mut self) -> bool {
        self.as_any().is::<T>()
    }
}
