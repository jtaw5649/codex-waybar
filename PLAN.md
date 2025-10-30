## Waybar Shimmer Requirements

- Shimmer band must appear **≈ four characters wide** by default, with width scaled via `mask_scale`. Width should never drop below two average glyph widths.
- Animation should behave like the text sits inside a **tunnel**:
  - The sweep must enter from beyond the left edge, fully traverse the glyphs, and exit beyond the right edge.
  - There must be **no sudden flash** when the sweep reaches either end; transparency should remain continuous throughout the loop.
- The shimmer effect has to maintain **full theme/background transparency** inside the sweep while the surrounding text remains fully opaque.
- Optional highlight (`overlay_scale`) should layer on top without altering transparency or causing visible seams.
- Module spacing must align with other Waybar modules (padding/margins identical to adjacent widgets).

### Abandoned Experiments

- **Clamp tunnel to 40 % of layout width** (2025-10-30): reduced `highlight_half` so DEST_OUT could not wipe all glyphs. Result: the active band shrank to ~1 px and text flashed when the sweep exited. Do **not** retry this approach; look for a method that maintains width while preventing DEST_OUT from erasing the full layout.

### Current Implementation (2025-10-30)

- Base glyphs render to a `CAIRO_CONTENT_ALPHA` mask surface via `cairo_surface_create_similar`, with the tunnel carved out using a translated linear gradient and `CAIRO_OPERATOR_DEST_OUT`.
- The widget paints the glyph color by calling `cairo_mask_surface`, so transparency comes solely from the mask—no DEST_OUT applied directly to the main context.
- A second alpha mask (optional) builds the shimmer highlight and is blended with `CAIRO_OPERATOR_SCREEN`, keeping the highlight aligned with the tunnel without washing out the text.
- Shadow depth now comes from directly stroking the glyph path: offset the layout, draw a wide rounded stroke with dark opacity, then clear a narrower stroke to leave a soft tunnel ring.

### Shadow Experiments

- **Edge shadow blend in highlight gradient** (2025-10-30): mixed a darker tint toward the feather region by modulating the highlight gradient’s alpha and color. Visual difference was negligible; revisit with a dedicated blur/offset shadow rather than color mixing.
- **Blurred shadow mask (2025-10-30)**: duplicated the highlight mask, applied a separable box blur, and composited the result as a darker offset. Needs heavier blur radius to become noticeable.
- **Heavy blur radius (~1.6× char width, 2025-10-30)**: increased radius/offset significantly but the shadow remained imperceptible. Treat this path as a dead end; look for an approach that introduces a separate soft-edge gradient or blur the glyph coverage before cutting the tunnel.
- **Gradient stroke shadow (2025-10-30)**: drew a wide stroked path with a dark-to-transparent gradient. Result appeared misaligned and visually harsh; abandon this approach.
- **Shadow mask blur (2025-10-30)**: duplicated the glyph alpha, removed the tunnel, blurred, and composited. No visible effect—likely too much alpha loss or offset; revisit with stronger levels or additive blend.
- **Wide-rim blur with cutout (2025-10-30)**: expanded mask, blurred, offset. Produced a global glow and display-dependent artifact; revert and pursue controlled dual-mask approach.
- **Dual-mask blur with trim (2025-10-30)**: blur + subtract approach still shows no discernible shadow. Need a fresh strategy (e.g., dedicated offscreen light map or additive gradient).
- **Stroke ring shadow (2025-10-30)**: offset path strokes create inconsistent glow (missing on primary display, global blur on secondary). Abandon this technique.
