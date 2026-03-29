//! Block type definitions and OSM tag → Minecraft block mappings.

use std::collections::HashMap;

/// Minecraft blocks used in world generation, stored as u8 for memory efficiency.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)] // enum variants reserved for future block mappings
#[allow(clippy::enum_variant_names)] // GrassBlock intentionally mirrors Minecraft naming
pub enum Block {
    Air = 0,
    Bedrock = 1,
    Stone = 2,
    Dirt = 3,
    GrassBlock = 4,
    Water = 5,
    Sand = 6,
    Gravel = 7,
    OakLog = 8,
    OakLeaves = 9,
    StoneBrick = 10,
    Concrete = 11,
    Cobblestone = 12,
    BlackConcrete = 13,
    GrayConcrete = 14,
    StoneSlab = 15,
    YellowConcrete = 16,
    OakSign = 17,
    GlassPane = 18,
    OakStairs = 19,
    OakSlab = 20,
    OakFence = 21,
    CobblestoneWall = 22,
    Brick = 23,
    Sandstone = 24,
    OakPlanks = 25,
    SprucePlanks = 26,
    WhiteConcrete = 27,
    StoneBrickStairs = 28,
    Rail = 29,
    TallGrass = 30,
    Fern = 31,
    Poppy = 32,
    Torch = 33,
    Lantern = 34,
    StoneBrickWall = 35,
    BirchLog = 36,
    BirchLeaves = 37,
    PolishedBlackstoneSlab = 38,
    SmoothStoneSlab = 39,
    AndesiteSlab = 40,
    CherrySign = 41,
    /// Full snow block — used as alpine sub-surface fill.
    Snow = 42,
    /// Thin snow layer (1/8th block) placed on top of stone at high altitude.
    SnowLayer = 43,
    /// Ice block — used for frozen water surfaces.
    Ice = 44,
    /// Hanging sign — used for address labels on buildings.
    CherryHangingSign = 45,
    /// Dispenser — used for mailbox POI decoration.
    Dispenser = 46,
    /// Brewing stand — used for cafe/coffee POI decoration.
    BrewingStand = 47,
    /// Bookshelf — used for library/school POI decoration.
    Bookshelf = 48,
    /// Cauldron — used for waste basket POI decoration.
    Cauldron = 49,
    /// Bed (red) — used for hotel/lodging POI decoration.
    Bed = 50,
    /// Furnace — used for restaurant POI decoration.
    Furnace = 51,
    /// Barrel — used for storage/shop POI decoration.
    Barrel = 52,
    /// Bell — used for church/worship POI decoration.
    Bell = 53,
    /// Campfire — used for fire station POI decoration.
    Campfire = 54,
    /// Hay bale — used for farm POI decoration.
    HayBale = 55,
}

impl Block {
    /// Bedrock Edition block identifier string.
    pub fn bedrock_name(self) -> &'static str {
        match self {
            Block::Air => "minecraft:air",
            Block::Bedrock => "minecraft:bedrock",
            Block::Stone => "minecraft:stone",
            Block::Dirt => "minecraft:dirt",
            Block::GrassBlock => "minecraft:grass_block",
            Block::Water => "minecraft:water",
            Block::Sand => "minecraft:sand",
            Block::Gravel => "minecraft:gravel",
            Block::OakLog => "minecraft:oak_log",
            Block::OakLeaves => "minecraft:oak_leaves",
            Block::StoneBrick => "minecraft:stone_bricks",
            Block::Concrete => "minecraft:light_gray_concrete",
            Block::Cobblestone => "minecraft:cobblestone",
            Block::BlackConcrete => "minecraft:black_concrete",
            Block::GrayConcrete => "minecraft:gray_concrete",
            Block::StoneSlab => "minecraft:stone_block_slab",
            Block::YellowConcrete => "minecraft:yellow_concrete",
            Block::OakSign => "minecraft:standing_sign",
            Block::GlassPane => "minecraft:glass_pane",
            Block::OakStairs => "minecraft:oak_stairs",
            Block::OakSlab => "minecraft:oak_slab",
            Block::OakFence => "minecraft:oak_fence",
            Block::CobblestoneWall => "minecraft:cobblestone_wall",
            Block::Brick => "minecraft:brick_block",
            Block::Sandstone => "minecraft:sandstone",
            Block::OakPlanks => "minecraft:oak_planks",
            Block::SprucePlanks => "minecraft:spruce_planks",
            Block::WhiteConcrete => "minecraft:white_concrete",
            Block::StoneBrickStairs => "minecraft:stone_brick_stairs",
            Block::Rail => "minecraft:rail",
            Block::TallGrass => "minecraft:tallgrass",
            Block::Fern => "minecraft:tallgrass",
            Block::Poppy => "minecraft:red_flower",
            Block::Torch => "minecraft:torch",
            Block::Lantern => "minecraft:lantern",
            Block::StoneBrickWall => "minecraft:cobblestone_wall",
            Block::BirchLog => "minecraft:birch_log",
            Block::BirchLeaves => "minecraft:birch_leaves",
            Block::PolishedBlackstoneSlab => "minecraft:polished_blackstone_slab",
            Block::SmoothStoneSlab => "minecraft:smooth_stone_slab",
            Block::AndesiteSlab => "minecraft:andesite_slab",
            Block::CherrySign => "minecraft:cherry_standing_sign",
            Block::Snow => "minecraft:snow",
            Block::SnowLayer => "minecraft:snow_layer",
            Block::Ice => "minecraft:ice",
            Block::CherryHangingSign => "minecraft:cherry_hanging_sign",
            Block::Dispenser => "minecraft:dispenser",
            Block::BrewingStand => "minecraft:brewing_stand",
            Block::Bookshelf => "minecraft:bookshelf",
            Block::Cauldron => "minecraft:cauldron",
            Block::Bed => "minecraft:bed",
            Block::Furnace => "minecraft:furnace",
            Block::Barrel => "minecraft:barrel",
            Block::Bell => "minecraft:bell",
            Block::Campfire => "minecraft:campfire",
            Block::HayBale => "minecraft:hay_block",
        }
    }

    /// Block states for the palette entry (e.g. sign direction, slab half, etc.).
    pub fn block_states(self) -> Vec<BlockState> {
        match self {
            Block::OakSign | Block::CherrySign => vec![BlockState::Int("ground_sign_direction", 0)],
            Block::TallGrass => vec![BlockState::String("tall_grass_type", "tall")],
            Block::Fern => vec![BlockState::String("tall_grass_type", "fern")],
            Block::Poppy => vec![BlockState::String("flower_type", "poppy")],
            Block::CobblestoneWall => {
                vec![BlockState::String("wall_block_type", "cobblestone")]
            }
            Block::StoneBrickWall => {
                vec![BlockState::String("wall_block_type", "stone_brick")]
            }
            Block::Torch => vec![BlockState::String("torch_facing_direction", "top")],
            Block::Lantern => vec![BlockState::Byte("hanging", 0)],
            Block::OakSlab | Block::PolishedBlackstoneSlab | Block::SmoothStoneSlab => {
                vec![BlockState::String("minecraft:vertical_half", "bottom")]
            }
            Block::AndesiteSlab => vec![BlockState::String("minecraft:vertical_half", "bottom")],
            Block::Sandstone => vec![BlockState::String("sand_stone_type", "default")],
            Block::BirchLog => vec![BlockState::String("pillar_axis", "y")],
            Block::BirchLeaves => vec![BlockState::Byte("persistent_bit", 1)],
            Block::OakLeaves => vec![BlockState::Byte("persistent_bit", 1)],
            Block::OakStairs => vec![
                BlockState::Int("weirdo_direction", 0),
                BlockState::Byte("upside_down_bit", 0),
            ],
            Block::StoneBrickStairs => vec![
                BlockState::Int("weirdo_direction", 0),
                BlockState::Byte("upside_down_bit", 0),
            ],
            Block::Rail => vec![BlockState::Int("rail_direction", 0)],
            Block::SnowLayer => vec![BlockState::Int("height", 0)],
            Block::CherryHangingSign => vec![
                BlockState::Byte("attached_bit", 0),
                BlockState::Int("facing_direction", 2),
                BlockState::Int("ground_sign_direction", 0),
                BlockState::Byte("hanging", 1),
            ],
            Block::Dispenser => vec![BlockState::Int("facing_direction", 1)], // facing up
            Block::Furnace => vec![BlockState::String("minecraft:cardinal_direction", "south")],
            Block::Barrel => vec![
                BlockState::Int("facing_direction", 1),
                BlockState::Byte("open_bit", 0),
            ],
            Block::Bell => vec![
                BlockState::String("attachment", "standing"),
                BlockState::Int("direction", 0),
                BlockState::Byte("toggle_bit", 0),
            ],
            Block::Campfire => vec![
                BlockState::String("minecraft:cardinal_direction", "south"),
                BlockState::Byte("extinguished", 0),
            ],
            Block::HayBale => vec![BlockState::String("pillar_axis", "y")],
            Block::Bed => vec![
                BlockState::Int("direction", 0),
                BlockState::Byte("head_piece_bit", 1),
                BlockState::Byte("occupied_bit", 0),
            ],
            _ => vec![],
        }
    }
}

/// Typed block state value for Bedrock Edition NBT palette entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum BlockState {
    Int(&'static str, i32),
    Byte(&'static str, i8),
    String(&'static str, &'static str),
}

// ── OSM tag → Block mappings ───────────────────────────────────────────────

/// Road definition: surface block, half-width, whether to add sidewalks and center line.
pub struct RoadStyle {
    pub surface: Block,
    pub sidewalk_surface: Block,
    pub half_width: i32,
    pub sidewalk: bool,
    #[allow(dead_code)] // reserved for future road center-line rendering
    pub center_line: bool,
    #[allow(dead_code)]
    pub edge_lines: bool,
}

/// Map `highway=*` value to a road style (block, width, sidewalks).
pub fn highway_to_style(highway_type: &str) -> RoadStyle {
    match highway_type {
        "motorway" | "trunk" => RoadStyle {
            surface: Block::PolishedBlackstoneSlab,
            sidewalk_surface: Block::SmoothStoneSlab,
            half_width: 3,
            sidewalk: false,
            center_line: true,
            edge_lines: false,
        },
        "primary" => RoadStyle {
            surface: Block::PolishedBlackstoneSlab,
            sidewalk_surface: Block::SmoothStoneSlab,
            half_width: 2,
            sidewalk: true,
            center_line: true,
            edge_lines: false,
        },
        "secondary" | "tertiary" => RoadStyle {
            surface: Block::PolishedBlackstoneSlab,
            sidewalk_surface: Block::SmoothStoneSlab,
            half_width: 2,
            sidewalk: true,
            center_line: false,
            edge_lines: false,
        },
        "residential" | "unclassified" | "living_street" | "service" => RoadStyle {
            surface: Block::PolishedBlackstoneSlab,
            sidewalk_surface: Block::SmoothStoneSlab,
            half_width: 2,
            sidewalk: true,
            center_line: false,
            edge_lines: false,
        },
        "path" | "footway" | "cycleway" | "track" | "pedestrian" => RoadStyle {
            surface: Block::AndesiteSlab,
            sidewalk_surface: Block::AndesiteSlab,
            half_width: 1,
            sidewalk: false,
            center_line: false,
            edge_lines: false,
        },
        _ => RoadStyle {
            surface: Block::PolishedBlackstoneSlab,
            sidewalk_surface: Block::SmoothStoneSlab,
            half_width: 1,
            sidewalk: false,
            center_line: false,
            edge_lines: false,
        },
    }
}

/// Map `landuse=*` value to a surface block.
pub fn landuse_to_block(landuse: &str) -> Block {
    match landuse {
        "forest" | "wood" => Block::OakLog,
        "grass" | "meadow" | "park" | "recreation_ground" | "village_green" => Block::GrassBlock,
        "farmland" | "farmyard" => Block::Dirt,
        "beach" | "sand" => Block::Sand,
        "reservoir" | "water" | "basin" => Block::Water,
        _ => Block::GrassBlock,
    }
}

/// Block used for building walls.
#[allow(dead_code)]
pub fn building_wall_block() -> Block {
    Block::StoneBrick
}

/// Block for `natural=*` features.
pub fn natural_to_block(natural: &str) -> Block {
    match natural {
        "water" | "bay" | "strait" => Block::Water,
        "beach" | "sand" => Block::Sand,
        "wood" => Block::OakLog,
        "grassland" | "heath" | "scrub" => Block::GrassBlock,
        "bare_rock" | "scree" | "cliff" => Block::Stone,
        _ => Block::GrassBlock,
    }
}

/// Map a surface block to the nearest Bedrock legacy biome ID (Data2D format).
///
/// Biome IDs used:
/// - 1  = plains        (grass, dirt, roads, buildings)
/// - 3  = extreme_hills (stone surfaces)
/// - 4  = forest        (oak trees)
/// - 7  = river         (water)
/// - 12 = ice_plains    (snow / alpine terrain)
/// - 16 = beach         (sand)
/// - 24 = deep_ocean    (ice blocks)
/// - 27 = birch_forest  (birch trees)
pub fn surface_to_biome(block: Block) -> u8 {
    match block {
        Block::Water => 7,
        Block::OakLog | Block::OakLeaves => 4,
        Block::BirchLog | Block::BirchLeaves => 27,
        Block::Sand => 16,
        Block::Stone => 3,
        Block::Snow | Block::SnowLayer => 12,
        Block::Ice => 24,
        _ => 1, // Default: plains. Covers roads, vegetation, structures, Air, and any future Block variants.
    }
}

/// Waterway definition: channel half-width and depth in blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterwayStyle {
    /// Half the channel width (0 = single-block trench).
    pub half_width: i32,
    /// Depth of the channel in blocks below surface.
    pub depth: i32,
}

/// Map `waterway=*` value (and optional OSM tags) to a waterway style.
///
/// OSM `width` and `depth` tags override type defaults when present and parseable.
/// `scale` is metres-per-block (from `ConvertParams::scale`).
pub fn waterway_to_style(
    waterway_type: &str,
    tags: &HashMap<String, String>,
    scale: f64,
) -> WaterwayStyle {
    // Type-based defaults
    let (default_hw, default_depth) = match waterway_type {
        "river" => (3, 4),
        "canal" => (2, 3),
        "stream" => (1, 2),
        "ditch" | "drain" => (0, 1),
        _ => (1, 2),
    };

    // OSM tag overrides (divide metres by scale, clamp)
    let half_width = tags
        .get("width")
        .and_then(|v| v.parse::<f64>().ok())
        .map(|w| ((w / scale / 2.0).round() as i32).clamp(0, 8))
        .unwrap_or(default_hw);

    let depth = tags
        .get("depth")
        .and_then(|v| v.parse::<f64>().ok())
        .map(|d| ((d / scale).round() as i32).clamp(1, 6))
        .unwrap_or(default_depth);

    WaterwayStyle { half_width, depth }
}

/// Choose a building wall block based on `building:material` tag.
pub fn building_block(tags: &HashMap<String, String>) -> Block {
    match tags.get("building:material").map(|s| s.as_str()) {
        Some("brick") => Block::Brick,
        Some("wood") | Some("timber") => Block::OakPlanks,
        Some("concrete") => Block::WhiteConcrete,
        Some("sandstone") => Block::Sandstone,
        Some("metal") => Block::GrayConcrete,
        Some("stone") => Block::StoneBrick,
        _ => Block::StoneBrick,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn building_block_brick() {
        let mut tags = HashMap::new();
        tags.insert("building:material".to_string(), "brick".to_string());
        assert_eq!(building_block(&tags), Block::Brick);
    }

    #[test]
    fn building_block_default() {
        let tags = HashMap::new();
        assert_eq!(building_block(&tags), Block::StoneBrick);
    }

    #[test]
    fn building_block_wood() {
        let mut tags = HashMap::new();
        tags.insert("building:material".to_string(), "wood".to_string());
        assert_eq!(building_block(&tags), Block::OakPlanks);
    }

    #[test]
    fn surface_to_biome_water() {
        assert_eq!(surface_to_biome(Block::Water), 7);
    }

    #[test]
    fn surface_to_biome_forest() {
        assert_eq!(surface_to_biome(Block::OakLog), 4);
        assert_eq!(surface_to_biome(Block::OakLeaves), 4);
    }

    #[test]
    fn surface_to_biome_birch() {
        assert_eq!(surface_to_biome(Block::BirchLog), 27);
        assert_eq!(surface_to_biome(Block::BirchLeaves), 27);
    }

    #[test]
    fn surface_to_biome_beach() {
        assert_eq!(surface_to_biome(Block::Sand), 16);
    }

    #[test]
    fn surface_to_biome_mountains() {
        assert_eq!(surface_to_biome(Block::Stone), 3);
    }

    #[test]
    fn surface_to_biome_plains_default() {
        // Grass, dirt, roads, buildings all → plains (biome 1)
        assert_eq!(surface_to_biome(Block::GrassBlock), 1);
        assert_eq!(surface_to_biome(Block::Dirt), 1);
        assert_eq!(surface_to_biome(Block::Concrete), 1);
        assert_eq!(surface_to_biome(Block::StoneBrick), 1);
        assert_eq!(surface_to_biome(Block::Cobblestone), 1);
        assert_eq!(surface_to_biome(Block::Gravel), 1);
    }

    #[test]
    fn waterway_style_river() {
        let tags = HashMap::new();
        let style = waterway_to_style("river", &tags, 1.0);
        assert_eq!(style.half_width, 3);
        assert_eq!(style.depth, 4);
    }

    #[test]
    fn waterway_style_canal() {
        let tags = HashMap::new();
        let style = waterway_to_style("canal", &tags, 1.0);
        assert_eq!(style.half_width, 2);
        assert_eq!(style.depth, 3);
    }

    #[test]
    fn waterway_style_stream() {
        let tags = HashMap::new();
        let style = waterway_to_style("stream", &tags, 1.0);
        assert_eq!(style.half_width, 1);
        assert_eq!(style.depth, 2);
    }

    #[test]
    fn waterway_style_ditch() {
        let tags = HashMap::new();
        let style = waterway_to_style("ditch", &tags, 1.0);
        assert_eq!(style.half_width, 0);
        assert_eq!(style.depth, 1);
    }

    #[test]
    fn waterway_style_drain() {
        let tags = HashMap::new();
        let style = waterway_to_style("drain", &tags, 1.0);
        assert_eq!(style.half_width, 0);
        assert_eq!(style.depth, 1);
    }

    #[test]
    fn waterway_style_default_fallback() {
        let tags = HashMap::new();
        let style = waterway_to_style("unknown_type", &tags, 1.0);
        assert_eq!(style.half_width, 1);
        assert_eq!(style.depth, 2);
    }

    #[test]
    fn waterway_style_width_tag_override() {
        let mut tags = HashMap::new();
        tags.insert("width".to_string(), "10.0".to_string());
        let style = waterway_to_style("stream", &tags, 1.0);
        assert_eq!(style.half_width, 5);
    }

    #[test]
    fn waterway_style_depth_tag_override() {
        let mut tags = HashMap::new();
        tags.insert("depth".to_string(), "6.0".to_string());
        let style = waterway_to_style("stream", &tags, 1.0);
        assert_eq!(style.depth, 6);
    }

    #[test]
    fn waterway_style_non_numeric_tags_ignored() {
        let mut tags = HashMap::new();
        tags.insert("width".to_string(), "narrow".to_string());
        tags.insert("depth".to_string(), "shallow".to_string());
        let style = waterway_to_style("river", &tags, 1.0);
        assert_eq!(style.half_width, 3);
        assert_eq!(style.depth, 4);
    }

    #[test]
    fn waterway_style_width_clamped() {
        let mut tags = HashMap::new();
        tags.insert("width".to_string(), "200.0".to_string());
        let style = waterway_to_style("river", &tags, 1.0);
        assert_eq!(style.half_width, 8);
    }

    #[test]
    fn waterway_style_depth_clamped_min() {
        let mut tags = HashMap::new();
        tags.insert("depth".to_string(), "0.0".to_string());
        let style = waterway_to_style("river", &tags, 1.0);
        assert_eq!(style.depth, 1);
    }

    #[test]
    fn waterway_style_scale_applied() {
        let mut tags = HashMap::new();
        tags.insert("width".to_string(), "4.0".to_string());
        let style = waterway_to_style("stream", &tags, 2.0);
        assert_eq!(style.half_width, 1);
    }

    #[test]
    fn waterway_style_depth_clamped_max() {
        let mut tags = HashMap::new();
        tags.insert("depth".to_string(), "100.0".to_string()); // far above max → clamped to 6
        let style = waterway_to_style("stream", &tags, 1.0);
        assert_eq!(style.depth, 6);
    }

    #[test]
    fn waterway_style_scale_applied_to_depth() {
        let mut tags = HashMap::new();
        tags.insert("depth".to_string(), "4.0".to_string()); // 4m at scale 2.0 → 2 blocks deep
        let style = waterway_to_style("stream", &tags, 2.0);
        assert_eq!(style.depth, 2);
    }
}
