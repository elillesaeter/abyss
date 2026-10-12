//! Compile-time guard: the shader source is embedded, so a typo in a binding name
//! or entry point would only surface when a GPU is present. This keeps the file
//! referenced from a test that runs anywhere.

#[cfg(test)]
mod tests {
    const SHADER: &str = include_str!("shader.wgsl");

    #[test]
    fn shader_declares_the_entry_points_and_bindings_the_pipeline_asks_for() {
        for needle in [
            "fn vs_main",
            "fn fs_main",
            "@group(0) @binding(0) var<uniform> view",
            "mask_band_y: f32",
            "block_tiles: f32",
            "@group(0) @binding(1) var atlas: texture_2d<f32>",
            "@group(0) @binding(2) var atlas_sampler: sampler",
            "@group(0) @binding(3) var light_map: texture_2d<f32>",
            "@group(0) @binding(4) var light_sampler: sampler",
            "light_origin: vec2<f32>",
            "light_size: vec2<f32>",
            "direct: vec4<f32>",
            "@group(0) @binding(5) var sun_map: texture_2d<f32>",
            "@location(0) instance_pos: vec2<f32>",
            "@location(1) instance_axis_x: vec2<f32>",
            "@location(2) instance_axis_y: vec2<f32>",
            "@location(3) instance_cells: vec2<u32>",
            "@location(4) instance_tint: vec4<f32>",
            "@location(5) instance_id: u32",
            "@location(6) instance_edges: u32",
            &format!("const EDGE_BLOCKY: u32 = {}u", crate::autotile::EDGE_BLOCKY),
            &format!("const LIT_SIDE: u32 = {}u", crate::autotile::LIT_SIDE),
            &format!("const SPRITE: u32 = {}u", crate::autotile::SPRITE),
            &format!("const SPRITE_BAND_Y: f32 = {:.1}", crate::atlas::SPRITE_BAND_Y as f32),
            &format!(
                "const FRAME: vec2<f32> = vec2<f32>({:.1}, {:.1})",
                crate::atlas::FRAME_W as f32,
                crate::atlas::FRAME_H as f32
            ),
            &format!("const FRAME_COLS: u32 = {}u", crate::atlas::FRAME_COLS),
            &format!("const UNLIT: u32 = {}u", crate::autotile::UNLIT),
            &format!("const GLASS: u32 = {}u", crate::autotile::GLASS),
            &format!("const ICON: u32 = {}u", crate::autotile::ICON),
            &format!("const ICON_BAND_Y: f32 = {:.1}", crate::atlas::ICON_BAND_Y as f32),
            &format!("const ICON_CELL: f32 = {:.1}", crate::atlas::ICON as f32),
            &format!("const ICON_COLS: u32 = {}u", crate::atlas::ICON_COLS),
            "fn fs_soft",
            "@location(1) id: u32",
        ] {
            assert!(SHADER.contains(needle), "shader.wgsl is missing `{needle}`");
        }
    }

    #[test]
    fn outline_shader_declares_what_the_outline_pipeline_binds() {
        const OUTLINE: &str = include_str!("outline.wgsl");
        for needle in [
            "fn vs_full",
            "fn fs_outline",
            "@group(0) @binding(0) var scene: texture_2d<f32>",
            "@group(0) @binding(1) var ids: texture_2d<u32>",
            "@group(0) @binding(2) var<uniform> outline: Outline",
            &format!("const ID_TERRAIN: u32 = {}u", crate::autotile::ID_TERRAIN),
            &format!("const NO_OUTLINE: u32 = {}u", crate::autotile::NO_OUTLINE),
        ] {
            assert!(OUTLINE.contains(needle), "outline.wgsl is missing `{needle}`");
        }
    }

    #[test]
    fn backdrop_shader_declares_what_the_backdrop_pipeline_binds() {
        const BACKDROP: &str = include_str!("backdrop.wgsl");
        for needle in [
            "fn vs_full",
            "fn fs_backdrop",
            "@group(0) @binding(0) var<uniform> backdrop: Backdrop",
            "@group(0) @binding(1) var strips: texture_2d<f32>",
            &format!("layers: array<Layer, {}>", crate::backdrop::LAYERS),
            &format!("const ART_PX: f32 = {}.0", crate::atlas::CELL),
        ] {
            assert!(BACKDROP.contains(needle), "backdrop.wgsl is missing `{needle}`");
        }
    }
}
