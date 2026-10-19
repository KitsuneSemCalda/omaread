//! Painting a page, and compositing more than one document into a frame.
//!
//! `blitz_paint::paint_scene` calls `scene.reset()` on entry (CONTEXT.md §9), so
//! for a long time the page was the only thing that could be painted: a second
//! call erased the first. But that function is generic over `impl PaintScene`,
//! and the trait is ours to implement — so a wrapper whose `reset` does nothing
//! lets a second document be painted *over* the first, and one that also
//! pre-multiplies every transform can put it somewhere else on the screen.
//!
//! That wrapper is [`Compose`], and it is the whole mechanism behind both the
//! reading HUD and the second column. §9 notes that a *layer* transform moves
//! the clip shape rather than the content; a transform applied to every drawing
//! command is a different thing, and it does move the content.

use anyrender::{Glyph, NormalizedCoord, Paint, PaintRef, PaintScene};
use blitz_html::HtmlDocument;
use blitz_dom::Point;
use kurbo::{Affine, Rect, Shape, Stroke};
use peniko::{BlendMode, Color, Fill, FontData, ImageBrush, ImageData, ImageQuality, StyleRef};
use std::cell::RefCell;
use std::sync::Arc;

/// Window geometry for one frame, in the units each part of the paint path
/// wants: physical pixels for the surface, CSS pixels for the page box.
#[derive(Clone, Copy)]
pub struct Frame {
    /// Physical pixels.
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// CSS pixels.
    pub margin: f32,
    pub page_height: f32,
}

/// Paint one column: the slice `[top, top + extent)` of the flow, at `x`, with
/// everything above and below masked back to the page ground.
///
/// `first` resets the scene, so the first column of a frame clears it and the
/// rest compose on top. The masks are per column because two columns are two
/// different pages, and a break lands where it lands — a single full-width band
/// would clip the longer column or leak the shorter one's next line.
///
/// `x` and `width` are in CSS pixels.
#[allow(clippy::too_many_arguments)]
pub fn column(
    scene: &mut impl PaintScene,
    doc: &mut HtmlDocument,
    top: f32,
    extent: f32,
    x: f32,
    width: f32,
    frame: &Frame,
    ground: Color,
    first: bool,
) {
    let Frame { height, scale, margin, page_height, .. } = *frame;
    let extent = extent.clamp(0.0, page_height);

    doc.set_viewport_scroll(Point { x: 0.0, y: (top - margin) as f64 });

    // Painting at column width keeps the document's own background inside the
    // column; the scene is otherwise identical to a one-column frame.
    let w = (width * scale as f32).round() as u32;
    if first {
        // The first column starts the frame, so blitz's reset goes through.
        let mut scene = Compose { inner: scene, shift: Affine::IDENTITY, reset: true };
        blitz_paint::paint_scene(&mut scene, doc, scale, w, height);
        soft_hyphens(&mut scene, doc, scale, height);
    } else {
        let shift = Affine::translate((x as f64 * scale, 0.0));
        let mut shifted = Compose { inner: scene, shift, reset: false };
        blitz_paint::paint_scene(&mut shifted, doc, scale, w, height);
        soft_hyphens(&mut shifted, doc, scale, height);
    }

    ground_band(scene, x, width, 0.0, margin, ground, frame);
    ground_band(scene, x, width, margin + extent, height as f32 / scale as f32, ground, frame);
}

/// Fill a horizontal band of one column with the page ground, in CSS pixels.
fn ground_band(
    scene: &mut impl PaintScene,
    x: f32,
    width: f32,
    y0: f32,
    y1: f32,
    ground: Color,
    frame: &Frame,
) {
    if y1 <= y0 {
        return;
    }
    let s = frame.scale;
    PaintScene::fill(
        scene,
        Fill::NonZero,
        Affine::IDENTITY,
        ground,
        None,
        &Rect::new(x as f64 * s, y0 as f64 * s, (x + width) as f64 * s, y1 as f64 * s),
    );
}

/// Blank the frame to the page ground.
///
/// A column painted with `first` resets the scene itself, but only at x = 0 —
/// during a page turn every column is offset, so the reset has to happen here
/// instead.
pub fn clear(scene: &mut impl PaintScene, ground: Color, frame: &Frame) {
    PaintScene::reset(scene);
    PaintScene::fill(
        scene,
        Fill::NonZero,
        Affine::IDENTITY,
        ground,
        None,
        &Rect::new(0.0, 0.0, frame.width as f64, frame.height as f64),
    );
}

/// Paint one page of a flow: the slice `[top, top + extent)`, with everything
/// above and below masked back to the page ground.
///
/// `extent` is the page's real height — `Pages::extent_of` — not the nominal
/// `page_height`. A break snaps up to a line boundary, so the two differ by up
/// to a line, and masking from the nominal height leaves the next page's first
/// line showing, sliced through the middle.
///
/// This resets the scene, so it goes first in a frame.
pub fn page(
    scene: &mut impl PaintScene,
    doc: &mut HtmlDocument,
    top: f32,
    extent: f32,
    frame: &Frame,
    ground: Color,
) {
    let Frame { width, height, scale, margin, page_height } = *frame;
    let extent = extent.clamp(0.0, page_height);

    // `viewport_scroll` is the only thing that moves painted content: a layer
    // transform repositions a clip shape but not the drawing commands inside it.
    // Offsetting by `top - margin` puts this page's first line just below the
    // top margin.
    doc.set_viewport_scroll(Point { x: 0.0, y: (top - margin) as f64 });

    // NOTE: paint_scene resets the scene, discarding anything drawn before it,
    // so the margin masks must come *after*.
    blitz_paint::paint_scene(scene, doc, scale, width, height);
    soft_hyphens(scene, doc, scale, height);

    // The flow continues above and below this page. Mask it, so the margins
    // show the neighbouring lines' halves as clean paper.
    let mut band = |y0: f64, y1: f64| {
        PaintScene::fill(
            scene,
            Fill::NonZero,
            Affine::IDENTITY,
            ground,
            None,
            &Rect::new(0.0, y0, width as f64, y1),
        );
    };
    band(0.0, margin as f64 * scale);
    band((margin + extent) as f64 * scale, height as f64);
}

/// Draw the hyphen at every line that breaks at a soft hyphen.
///
/// Parley breaks at U+00AD and draws nothing there — it has no hyphenation —
/// so a hyphenated word read "Eng / lish". The hyphen is drawn here instead,
/// after the line's last glyph, in that run's font, size and colour, the way
/// `blitz_paint` draws the run itself. It hangs past the measure by its own
/// width: parley justified the line without it and cannot be asked for room.
///
/// Before the margin masks, so a hyphen on a neighbouring page is masked with
/// its line.
pub fn soft_hyphens(scene: &mut impl PaintScene, doc: &blitz_dom::BaseDocument, scale: f64, height: u32) {
    use blitz_dom::util::ToColorColor;
    use parley::PositionedLayoutItem;

    fn walk(
        scene: &mut impl PaintScene,
        dom: &blitz_dom::BaseDocument,
        id: usize,
        at: (f64, f64),
        scale: f64,
        height: f64,
    ) {
        let Some(node) = dom.get_node(id) else { return };
        let l = &node.final_layout;
        let here = (at.0 + l.location.x as f64, at.1 + l.location.y as f64);
        let tl = match &node.data {
            blitz_dom::NodeData::Element(el) => el.inline_layout_data.as_deref(),
            _ => None,
        };
        let on_screen = here.1 * scale < height && (here.1 + l.size.height as f64) * scale > 0.0;
        if let Some(tl) = tl.filter(|tl| on_screen && tl.text.contains('\u{AD}')) {
            // The layout's origin is the content box, as in `blitz_paint`.
            let origin = (
                here.0 + (l.border.left + l.padding.left) as f64,
                here.1 + (l.border.top + l.padding.top) as f64,
            );
            let transform = Affine::translate((origin.0 * scale, origin.1 * scale));
            for line in tl.layout.lines() {
                if !tl.text.get(line.text_range()).is_some_and(|t| t.ends_with('\u{AD}')) {
                    continue;
                }
                let Some(run) = line
                    .items()
                    .filter_map(|i| match i {
                        PositionedLayoutItem::GlyphRun(g) => Some(g),
                        _ => None,
                    })
                    .last()
                else {
                    continue;
                };
                let font = run.run().font();
                let Some(glyph) = hyphen_glyph(font) else { continue };
                let color = dom
                    .get_node(run.style().brush.id)
                    .and_then(|n| n.primary_styles())
                    .map(|s| s.get_inherited_text().color.as_color_color())
                    .unwrap_or(Color::BLACK);
                let skew = run
                    .run()
                    .synthesis()
                    .skew()
                    .map(|a| Affine::skew(a.to_radians().tan() as f64, 0.0));
                scene.draw_glyphs(
                    font,
                    run.run().font_size(),
                    true,
                    run.run().normalized_coords(),
                    Fill::NonZero,
                    &anyrender::Paint::from(color),
                    1.0,
                    transform,
                    skew,
                    std::iter::once(Glyph {
                        id: glyph,
                        x: run.offset() + run.advance(),
                        y: run.baseline(),
                    }),
                );
            }
        }
        for c in &node.children {
            walk(scene, dom, *c, here, scale, height);
        }
    }

    let scroll = doc.viewport_scroll();
    walk(scene, doc, 0, (-scroll.x, -scroll.y), scale, height as f64);
}

/// The font's hyphen: U+2010 where it has one, the hyphen-minus otherwise.
fn hyphen_glyph(font: &FontData) -> Option<u32> {
    use skrifa::MetadataProvider;
    let f = skrifa::FontRef::from_index(font.data.as_ref(), font.index).ok()?;
    let map = f.charmap();
    map.map('\u{2010}').or_else(|| map.map('-')).map(|g| g.to_u32())
}

/// Stroke a rounded outline around a rectangle given in CSS pixels.
///
/// The library's selection is painted rather than marked up: baking a
/// `.selected` class into the grid meant rebuilding the document on every arrow
/// key, and rebuilding re-requests every cover — 358 JPEG decodes, 1.4s, per
/// keypress. An outline also does not tint the cover it sits on.
pub fn outline(
    scene: &mut impl PaintScene,
    rect: (f32, f32, f32, f32),
    color: Color,
    width: f64,
    scale: f64,
) {
    let (x, y, w, h) = rect;
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    // Square, as Omarchy draws everything (Hyprland's `rounding = 0`).
    let r = kurbo::Rect::new(
        x as f64 * scale,
        y as f64 * scale,
        (x + w) as f64 * scale,
        (y + h) as f64 * scale,
    );
    PaintScene::stroke(
        scene,
        &Stroke::new(width * scale),
        Affine::IDENTITY,
        color,
        None,
        &r,
    );
}

/// Fill a rectangle given in CSS pixels — the wash under the pointer.
///
/// Painted *over* the surface, translucent, for the same reason the outline is
/// painted at all: a `:hover` class would mean rebuilding the document on every
/// mouse move. The HUD's own ground is opaque, so a wash beneath it would not
/// show at all.
pub fn wash(scene: &mut impl PaintScene, rect: (f32, f32, f32, f32), color: Color, scale: f64) {
    let (x, y, w, h) = rect;
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    // Square, as Omarchy draws everything (Hyprland's `rounding = 0`).
    let r = kurbo::Rect::new(
        x as f64 * scale,
        y as f64 * scale,
        (x + w) as f64 * scale,
        (y + h) as f64 * scale,
    );
    PaintScene::fill(scene, Fill::NonZero, Affine::IDENTITY, color, None, &r);
}

/// Which glyph a HUD control wants. The bundled faces carry no symbol glyphs
/// (checked: no ☰, no ⚑, no ✎ in Literata, Charis SIL or IBM Plex Mono), and a
/// missing glyph is visible tofu — so the icons are drawn, not typeset.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    /// A pennant: bookmark.
    Bookmark,
    /// Three rules: contents.
    Contents,
    /// A marker stroke: highlight.
    Highlight,
    /// A left-pointing triangle: back to the library.
    Back,
    /// A cog: settings.
    Gear,
}

/// Draw an icon inside a rectangle given in CSS pixels.
///
/// Shapes are described in a unit box and scaled, so they stay sharp at any
/// window scale and follow whatever colour the chrome is using.
pub fn icon(scene: &mut impl PaintScene, which: Icon, rect: (f32, f32, f32, f32), color: Color, scale: f64) {
    let (rx, ry, rw, rh) = rect;
    if rw <= 0.0 || rh <= 0.0 {
        return;
    }
    // Square, centred, and inset a little so it sits with the cap height.
    let side = rw.min(rh);
    let (ox, oy) = (rx + (rw - side) / 2.0, ry + (rh - side) / 2.0);
    let at = |ux: f64, uy: f64| {
        ((ox as f64 + ux * side as f64) * scale, (oy as f64 + uy * side as f64) * scale)
    };
    let mut path = kurbo::BezPath::new();

    match which {
        Icon::Bookmark => {
            for (i, (ux, uy)) in
                [(0.22, 0.08), (0.78, 0.08), (0.78, 0.92), (0.5, 0.68), (0.22, 0.92)]
                    .into_iter()
                    .enumerate()
            {
                let p = at(ux, uy);
                match i {
                    0 => path.move_to(p),
                    _ => path.line_to(p),
                }
            }
            path.close_path();
        }
        Icon::Contents => {
            for uy in [0.2, 0.47, 0.74] {
                let (x0, y0) = at(0.14, uy);
                let (x1, y1) = at(0.86, uy + 0.13);
                path.move_to((x0, y0));
                path.line_to((x1, y0));
                path.line_to((x1, y1));
                path.line_to((x0, y1));
                path.close_path();
            }
        }
        Icon::Highlight => {
            // A diagonal marker, with the stroke it just laid down beneath it.
            for (i, (ux, uy)) in
                [(0.10, 0.62), (0.62, 0.10), (0.82, 0.30), (0.30, 0.82)].into_iter().enumerate()
            {
                let p = at(ux, uy);
                match i {
                    0 => path.move_to(p),
                    _ => path.line_to(p),
                }
            }
            path.close_path();
            let (x0, y0) = at(0.10, 0.86);
            let (x1, y1) = at(0.90, 0.99);
            path.move_to((x0, y0));
            path.line_to((x1, y0));
            path.line_to((x1, y1));
            path.line_to((x0, y1));
            path.close_path();
        }
        Icon::Back => {
            for (i, (ux, uy)) in
                [(0.68, 0.12), (0.68, 0.88), (0.22, 0.5)].into_iter().enumerate()
            {
                let p = at(ux, uy);
                match i {
                    0 => path.move_to(p),
                    _ => path.line_to(p),
                }
            }
            path.close_path();
        }
        // Eight teeth around a hub, then a hole punched through the middle:
        // NonZero would fill the hole, so the ring is wound the other way.
        Icon::Gear => {
            let teeth = 8;
            for i in 0..teeth * 2 {
                let wide = i % 2 == 0;
                let r = if wide { 0.5 } else { 0.34 };
                let a0 = (i as f64) * std::f64::consts::TAU / (teeth as f64 * 2.0);
                let a1 = a0 + std::f64::consts::TAU / (teeth as f64 * 2.0);
                for (n, a) in [a0, a1].into_iter().enumerate() {
                    let p = at(0.5 + r * a.cos(), 0.5 + r * a.sin());
                    match i == 0 && n == 0 {
                        true => path.move_to(p),
                        false => path.line_to(p),
                    }
                }
            }
            path.close_path();
            let hole = 0.16;
            for i in 0..=12 {
                // Clockwise, against the teeth, so the even-odd of the winding
                // leaves the middle empty.
                let a = -(i as f64) * std::f64::consts::TAU / 12.0;
                let p = at(0.5 + hole * a.cos(), 0.5 + hole * a.sin());
                match i {
                    0 => path.move_to(p),
                    _ => path.line_to(p),
                }
            }
            path.close_path();
        }
    }

    PaintScene::fill(scene, Fill::NonZero, Affine::IDENTITY, color, None, &path);
}

/// Fill flow-coordinate rectangles over the page: a live selection, or a stored
/// highlight.
///
/// Drawn after the page, so it sits on top of the glyphs — which is what a
/// highlighter pen does. Rectangles are clipped to the page band, or a highlight
/// belonging to the next page would bleed into this one's margin.
#[allow(clippy::too_many_arguments)]
pub fn bands(
    scene: &mut impl PaintScene,
    rects: &[(f32, f32, f32, f32)],
    color: Color,
    top: f32,
    extent: f32,
    dx: f32,
    frame: &Frame,
) {
    let Frame { scale, margin, .. } = *frame;
    let (lo, hi) = (margin, margin + extent.min(frame.page_height));

    for &(x0, y0, x1, y1) in rects {
        // Flow -> screen, the same shift the page itself gets.
        let (sy0, sy1) = (y0 - top + margin, y1 - top + margin);
        let (cy0, cy1) = (sy0.max(lo), sy1.min(hi));
        if cy1 <= cy0 || x1 <= x0 {
            continue;
        }
        PaintScene::fill(
            scene,
            Fill::NonZero,
            Affine::IDENTITY,
            color,
            None,
            &Rect::new(
                ((x0 + dx) as f64) * scale,
                (cy0 as f64) * scale,
                ((x1 + dx) as f64) * scale,
                (cy1 as f64) * scale,
            ),
        );
    }
}

/// Paint a second document over whatever is already in the scene.
///
/// The document is laid out at window size and positions itself with margins,
/// so nothing here needs to know where the overlay wants to sit.
pub fn overlay(scene: &mut impl PaintScene, doc: &mut HtmlDocument, frame: &Frame) {
    doc.set_viewport_scroll(Point { x: 0.0, y: 0.0 });
    let mut scene = Compose { inner: scene, shift: Affine::IDENTITY, reset: false };
    blitz_paint::paint_scene(&mut scene, doc, frame.scale, frame.width, frame.height);
}

/// A `PaintScene` that composes instead of replacing: it swallows `reset` and
/// pre-multiplies `shift` into every transform it forwards.
///
/// Swallowing the reset is what lets a second `paint_scene` land on top of the
/// first while `BlitzDomPainter`'s fields stay `pub(crate)`. The shift is what
/// puts it somewhere else — the second column, or nothing at all for an overlay.
///
/// Every image goes through it too, and is drawn by [`fitted`]: resampled once
/// to the size it is painted at, then copied 1:1.
struct Compose<'a, S: PaintScene> {
    inner: &'a mut S,
    shift: Affine,
    /// Pass `reset` on: only for the first column of a frame.
    reset: bool,
}

impl<S: PaintScene> PaintScene for Compose<'_, S> {
    /// Half the point.
    fn reset(&mut self) {
        if self.reset {
            self.inner.reset();
        }
    }

    fn push_layer(
        &mut self,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
    ) {
        self.inner.push_layer(blend, alpha, self.shift * transform, clip);
    }

    fn pop_layer(&mut self) {
        self.inner.pop_layer();
    }

    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.inner.stroke(style, self.shift * transform, brush, brush_transform, shape);
    }

    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let transform = self.shift * transform;
        let brush = brush.into();
        // An image drawn whole, scaled but not rotated: what blitz does for
        // every `<img>`. Anything else goes through untouched.
        if let Paint::Image(img) = &brush
            && brush_transform.is_none()
            && let [sx, 0.0, 0.0, sy, x, y] = transform.as_coeffs()
            && sx > 0.0
            && sy > 0.0
            && img.image.format == peniko::ImageFormat::Rgba8
            && shape.as_rect() == Some(Rect::new(0.0, 0.0, img.image.width as f64, img.image.height as f64))
        {
            let (w, h) = (
                (img.image.width as f64 * sx).round() as u32,
                (img.image.height as f64 * sy).round() as u32,
            );
            if w > 0 && h > 0 {
                let image = fitted(img.image, w, h);
                let sampler = peniko::ImageSampler { quality: ImageQuality::Low, ..img.sampler };
                self.inner.fill(
                    style,
                    Affine::translate((x.round(), y.round())),
                    ImageBrush { image: &image, sampler },
                    None,
                    &Rect::new(0.0, 0.0, w as f64, h as f64),
                );
                return;
            }
        }
        self.inner.fill(style, transform, brush, brush_transform, shape);
    }

    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        style: impl Into<StyleRef<'a>>,
        brush: impl Into<PaintRef<'a>>,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph>,
    ) {
        let transform = self.shift * transform;
        self.inner.draw_glyphs(
            font,
            font_size,
            hint,
            normalized_coords,
            style,
            brush,
            brush_alpha,
            transform,
            glyph_transform,
            glyphs,
        );
    }

    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        brush: Color,
        radius: f64,
        std_dev: f64,
    ) {
        self.inner.draw_box_shadow(self.shift * transform, rect, brush, radius, std_dev);
    }
}

/// How many resampled images to keep. A page shows a few; this covers both
/// pages of a turn and the way back.
const FITTED_MAX: usize = 16;

thread_local! {
    /// Images resampled to the size they are drawn at, most recent first. The
    /// source is held weakly: a book image dies with its chapter, and an entry
    /// whose source is gone is never matched, so a new image that happens to
    /// reuse the old one's address cannot be drawn with the old pixels.
    static FITTED: RefCell<Vec<(peniko::WeakBlob<u8>, ImageData)>> = const { RefCell::new(Vec::new()) };
}

/// `src` resampled to exactly `w`×`h`, from the cache when it was asked for
/// before.
///
/// vello_cpu samples an image bilinearly per pixel per frame, and anyrender
/// converts the whole source every frame too: a page with a figure painted in
/// 30–95ms against 6ms for text, and a page turn paints two pages a frame, so
/// the slide stuttered whenever an image came into view. Resampled once, with a
/// proper filter, and drawn 1:1 with nearest sampling, it costs what text
/// does — and loses nothing, since 1:1 nearest is exact. The same move as the
/// library's covers (`net::FitCover`), made at paint time because a book
/// image's size is only known after layout: measure, columns, font size and
/// window all change it.
fn fitted(src: &ImageData, w: u32, h: u32) -> ImageData {
    if (w, h) == (src.width, src.height) {
        return src.clone();
    }
    let ptr = src.data.data().as_ptr();
    let hit = FITTED.with_borrow_mut(|cache| {
        cache.retain(|(weak, _)| weak.upgrade().is_some());
        let at = cache.iter().position(|(weak, img)| {
            (img.width, img.height) == (w, h)
                && weak.upgrade().is_some_and(|b| b.data().as_ptr() == ptr)
        })?;
        let entry = cache.remove(at);
        let image = entry.1.clone();
        cache.insert(0, entry);
        Some(image)
    });
    if let Some(image) = hit {
        return image;
    }

    let Some(view) = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(src.width, src.height, src.data.data())
    else {
        return src.clone();
    };
    let out = match w <= src.width && h <= src.height {
        // Area averaging: the right filter for shrinking, and the fast one.
        true => image::imageops::thumbnail(&view, w, h),
        false => image::imageops::resize(&view, w, h, image::imageops::FilterType::Triangle),
    };
    let image = ImageData {
        data: peniko::Blob::new(Arc::new(out.into_raw())),
        width: w,
        height: h,
        ..src.clone()
    };
    FITTED.with_borrow_mut(|cache| {
        cache.insert(0, (src.data.downgrade(), image.clone()));
        cache.truncate(FITTED_MAX);
    });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, px: u8) -> ImageData {
        ImageData {
            data: peniko::Blob::new(Arc::new(vec![px; (w * h * 4) as usize])),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: w,
            height: h,
        }
    }

    /// An image is resampled once to the size it is drawn at, then reused —
    /// resampling per frame would put back the cost this exists to remove.
    #[test]
    fn an_image_is_fitted_once_and_reused() {
        let src = image(400, 200, 200);
        let a = fitted(&src, 100, 50);
        assert_eq!((a.width, a.height), (100, 50));
        assert_eq!(a.data.data()[0], 200, "a flat image stays flat");
        let b = fitted(&src, 100, 50);
        assert_eq!(a.data.id(), b.data.id(), "the second draw is a cache hit");
        // A new size is a new entry, and drawn at its own size nothing is done.
        assert_eq!(fitted(&src, 200, 100).width, 200);
        assert_eq!(fitted(&src, 400, 200).data.id(), src.data.id());
    }

    /// A chapter's images die with it; the cache must not hand their pixels
    /// to whatever image comes next.
    #[test]
    fn a_dropped_image_is_never_matched() {
        let gone = image(64, 64, 10);
        fitted(&gone, 32, 32);
        drop(gone);
        let next = image(64, 64, 250);
        assert_eq!(fitted(&next, 32, 32).data.data()[0], 250);
    }

    /// Painted through the page path, a scaled image arrives at its painted
    /// size, unscaled — the whole point.
    #[test]
    fn a_scaled_image_is_drawn_one_to_one() {
        struct Spy(Vec<(Affine, u32, u32)>);
        impl PaintScene for Spy {
            fn reset(&mut self) {}
            fn push_layer(&mut self, _: impl Into<BlendMode>, _: f32, _: Affine, _: &impl Shape) {}
            fn pop_layer(&mut self) {}
            fn stroke<'a>(&mut self, _: &Stroke, _: Affine, _: impl Into<PaintRef<'a>>, _: Option<Affine>, _: &impl Shape) {}
            fn fill<'a>(&mut self, _: Fill, t: Affine, brush: impl Into<PaintRef<'a>>, _: Option<Affine>, _: &impl Shape) {
                if let Paint::Image(img) = brush.into() {
                    self.0.push((t, img.image.width, img.image.height));
                }
            }
            fn draw_glyphs<'a, 's: 'a>(
                &'s mut self, _: &'a FontData, _: f32, _: bool, _: &'a [NormalizedCoord],
                _: impl Into<StyleRef<'a>>, _: impl Into<PaintRef<'a>>, _: f32, _: Affine,
                _: Option<Affine>, _: impl Iterator<Item = Glyph>,
            ) {}
            fn draw_box_shadow(&mut self, _: Affine, _: Rect, _: Color, _: f64, _: f64) {}
        }

        let mut spy = Spy(Vec::new());
        let src = image(1000, 500, 0);
        let brush = ImageBrush { image: &src, sampler: peniko::ImageSampler::default() };
        let mut scene = Compose { inner: &mut spy, shift: Affine::translate((10.0, 0.0)), reset: false };
        scene.draw_image(brush, Affine::translate((5.3, 7.0)).pre_scale(0.5));
        assert_eq!(spy.0, vec![(Affine::translate((15.0, 7.0)), 500, 250)]);
    }
}
