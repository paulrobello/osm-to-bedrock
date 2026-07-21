//! Block type definitions and OSM tag → Minecraft block mappings.

use std::collections::HashMap;

use crate::osm::TagMap;

/// Minecraft blocks used in world generation, stored as u8 for memory efficiency.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)] // enum variants reserved for future block mappings
#[allow(clippy::enum_variant_names)] // GrassBlock intentionally mirrors Minecraft naming
pub enum Block {
    // === Terrain: air, subsurface, ground cover, water, trees ===
    /// Empty cell — the default block state.
    Air = 0,
    /// Bottommost layer of every chunk column.
    Bedrock = 1,
    /// Sub-surface fill below the dirt layer.
    Stone = 2,
    /// Layer between stone and the surface grass block.
    Dirt = 3,
    /// Default surface block for grassland/park land use.
    GrassBlock = 4,
    /// Water bodies (lakes, rivers, reservoirs, oceans).
    Water = 5,
    /// Beaches, deserts, sand land use.
    Sand = 6,
    /// Gravel surface (rare).
    Gravel = 7,
    /// Tree trunk for oak/birch forests and `landuse=forest`.
    OakLog = 8,
    /// Canopy block placed above oak logs.
    OakLeaves = 9,

    // === Building & road surface materials ===
    /// Building wall material.
    StoneBrick = 10,
    /// Light-gray concrete — generic building wall.
    Concrete = 11,
    /// Cobblestone road surface (older road class).
    Cobblestone = 12,
    /// Black concrete — asphalt-style road surface.
    BlackConcrete = 13,
    /// Gray concrete — road surface accent.
    GrayConcrete = 14,
    /// Stone slab — road surface / step block.
    StoneSlab = 15,
    /// Yellow concrete — road center-line / edge-line marker.
    YellowConcrete = 16,
    /// Oak standing sign — street-name and address signs.
    OakSign = 17,
    /// Glass pane — window feature on buildings.
    GlassPane = 18,
    /// Oak stairs — building stair feature.
    OakStairs = 19,
    /// Oak slab — building floor / step.
    OakSlab = 20,
    /// Oak fence — building perimeter / barrier.
    OakFence = 21,
    /// Cobblestone wall — barrier / boundary.
    CobblestoneWall = 22,
    /// Brick wall material for some building styles.
    Brick = 23,
    /// Sandstone building material.
    Sandstone = 24,
    /// Oak planks — wooden building wall/floor.
    OakPlanks = 25,
    /// Spruce planks — alternative wooden building material.
    SprucePlanks = 26,
    /// White concrete — modern building wall.
    WhiteConcrete = 27,
    /// Stone-brick stairs — stair variant for stone buildings.
    StoneBrickStairs = 28,

    // === Railways ===
    /// Rail track block for `railway=*` ways.
    Rail = 29,

    // === Decoration: vegetation, lighting, walls, slabs, signs ===
    /// Tall grass decoration on grass surfaces.
    TallGrass = 30,
    /// Fern decoration (alternate grass).
    Fern = 31,
    /// Poppy flower decoration.
    Poppy = 32,
    /// Torch — small light decoration.
    Torch = 33,
    /// Lantern — light decoration along roads.
    Lantern = 34,
    /// Stone-brick wall — decorative wall variant.
    StoneBrickWall = 35,
    /// Tree trunk for birch forests.
    BirchLog = 36,
    /// Canopy block placed above birch logs.
    BirchLeaves = 37,
    /// Polished blackstone slab — primary road surface for motorways and arterials.
    PolishedBlackstoneSlab = 38,
    /// Smooth stone slab — sidewalk surface paired with `PolishedBlackstoneSlab` roads.
    SmoothStoneSlab = 39,
    /// Andesite slab — path/footway/cycleway surface.
    AndesiteSlab = 40,
    /// Cherry wood standing sign — alternate sign variant for some POI decorations.
    CherrySign = 41,

    // === Climate: snow & ice ===
    /// Full snow block — used as alpine sub-surface fill.
    Snow = 42,
    /// Thin snow layer (1/8th block) placed on top of stone at high altitude.
    SnowLayer = 43,
    /// Ice block — used for frozen water surfaces.
    Ice = 44,

    // === POI decoration (placed at amenity/shop nodes) ===
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

    /// Java Edition block identifier string.
    pub fn java_name(self) -> &'static str {
        match self {
            Block::OakSign => "minecraft:oak_sign",
            Block::Brick => "minecraft:bricks",
            Block::StoneSlab => "minecraft:stone_slab",
            Block::Poppy => "minecraft:poppy",
            Block::TallGrass => "minecraft:tall_grass",
            Block::Fern => "minecraft:fern",
            Block::CherrySign => "minecraft:cherry_sign",
            Block::StoneBrickWall => "minecraft:stone_brick_wall",
            Block::SnowLayer => "minecraft:snow",
            // All other blocks share the same name between Bedrock and Java.
            _ => self.bedrock_name(),
        }
    }

    /// Java Edition block state properties as key-value string pairs.
    pub fn java_block_states(self) -> Vec<(&'static str, &'static str)> {
        match self {
            Block::OakSign | Block::CherrySign => vec![("rotation", "0")],
            Block::TallGrass | Block::Fern | Block::Poppy | Block::Torch => vec![],
            Block::CobblestoneWall => vec![("up", "true")],
            Block::StoneBrickWall => vec![("up", "true")],
            Block::OakSlab
            | Block::PolishedBlackstoneSlab
            | Block::SmoothStoneSlab
            | Block::AndesiteSlab => vec![("type", "bottom")],
            Block::BirchLog => vec![("axis", "y")],
            Block::OakLeaves => vec![("persistent", "true")],
            Block::BirchLeaves => vec![("persistent", "true")],
            Block::OakStairs | Block::StoneBrickStairs => vec![
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
            ],
            Block::Rail => vec![("shape", "north_south")],
            Block::SnowLayer => vec![("layers", "1")],
            Block::Dispenser => vec![("facing", "up")],
            Block::Furnace => vec![("facing", "south")],
            Block::Barrel => vec![("facing", "up"), ("open", "false")],
            Block::Bell => vec![("attachment", "floor"), ("facing", "north")],
            Block::Campfire => vec![("facing", "south"), ("lit", "true")],
            Block::Bed => vec![("facing", "north"), ("part", "head")],
            Block::Lantern => vec![("hanging", "false")],
            Block::HayBale => vec![("axis", "y")],
            Block::CherryHangingSign => vec![("attached", "false"), ("rotation", "0")],
            _ => vec![],
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

    /// Parse a block by its enum variant name (exact PascalCase), e.g. `"OakLog"`.
    ///
    /// Used by the custom block-mapping loader to resolve user-supplied names.
    /// The authoritative variant/name list lives in `ALL_BLOCK_VARIANTS` in the
    /// tests below — keep this match in sync with it.
    pub fn from_name(name: &str) -> Option<Block> {
        Some(match name {
            "Air" => Block::Air,
            "Bedrock" => Block::Bedrock,
            "Stone" => Block::Stone,
            "Dirt" => Block::Dirt,
            "GrassBlock" => Block::GrassBlock,
            "Water" => Block::Water,
            "Sand" => Block::Sand,
            "Gravel" => Block::Gravel,
            "OakLog" => Block::OakLog,
            "OakLeaves" => Block::OakLeaves,
            "StoneBrick" => Block::StoneBrick,
            "Concrete" => Block::Concrete,
            "Cobblestone" => Block::Cobblestone,
            "BlackConcrete" => Block::BlackConcrete,
            "GrayConcrete" => Block::GrayConcrete,
            "StoneSlab" => Block::StoneSlab,
            "YellowConcrete" => Block::YellowConcrete,
            "OakSign" => Block::OakSign,
            "GlassPane" => Block::GlassPane,
            "OakStairs" => Block::OakStairs,
            "OakSlab" => Block::OakSlab,
            "OakFence" => Block::OakFence,
            "CobblestoneWall" => Block::CobblestoneWall,
            "Brick" => Block::Brick,
            "Sandstone" => Block::Sandstone,
            "OakPlanks" => Block::OakPlanks,
            "SprucePlanks" => Block::SprucePlanks,
            "WhiteConcrete" => Block::WhiteConcrete,
            "StoneBrickStairs" => Block::StoneBrickStairs,
            "Rail" => Block::Rail,
            "TallGrass" => Block::TallGrass,
            "Fern" => Block::Fern,
            "Poppy" => Block::Poppy,
            "Torch" => Block::Torch,
            "Lantern" => Block::Lantern,
            "StoneBrickWall" => Block::StoneBrickWall,
            "BirchLog" => Block::BirchLog,
            "BirchLeaves" => Block::BirchLeaves,
            "PolishedBlackstoneSlab" => Block::PolishedBlackstoneSlab,
            "SmoothStoneSlab" => Block::SmoothStoneSlab,
            "AndesiteSlab" => Block::AndesiteSlab,
            "CherrySign" => Block::CherrySign,
            "Snow" => Block::Snow,
            "SnowLayer" => Block::SnowLayer,
            "Ice" => Block::Ice,
            "CherryHangingSign" => Block::CherryHangingSign,
            "Dispenser" => Block::Dispenser,
            "BrewingStand" => Block::BrewingStand,
            "Bookshelf" => Block::Bookshelf,
            "Cauldron" => Block::Cauldron,
            "Bed" => Block::Bed,
            "Furnace" => Block::Furnace,
            "Barrel" => Block::Barrel,
            "Bell" => Block::Bell,
            "Campfire" => Block::Campfire,
            "HayBale" => Block::HayBale,
            _ => return None,
        })
    }
}

/// Typed block state value for Bedrock Edition NBT palette entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BlockState {
    Int(&'static str, i32),
    Byte(&'static str, i8),
    String(&'static str, &'static str),
}

/// User-supplied overrides for the OSM tag → Block mappings.
///
/// Each map is keyed by the OSM tag *value*:
/// - `building`: the `building:material` value
/// - `highway`: the `highway` value (overrides the road **surface** block only)
/// - `landuse`: the `landuse` value
/// - `natural`: the `natural` value
///
/// An empty map (the `Default`) means "no overrides for this category" and the
/// built-in mapping is used. Loaded from a YAML file via
/// [`crate::block_mapping::load_block_overrides`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockOverrides {
    pub building: HashMap<String, Block>,
    pub highway: HashMap<String, Block>,
    pub landuse: HashMap<String, Block>,
    pub natural: HashMap<String, Block>,
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
    pub edge_lines: bool,
}

/// Map `highway=*` value to a road style (block, width, sidewalks).
///
/// When `ov` contains an entry for `highway_type`, its block replaces the road
/// **surface** only; width, sidewalk, and flags come from the built-in default.
pub fn highway_to_style(highway_type: &str, ov: Option<&BlockOverrides>) -> RoadStyle {
    let mut style = default_highway_to_style(highway_type);
    if let Some(o) = ov
        && let Some(&surface) = o.highway.get(highway_type)
    {
        style.surface = surface;
    }
    style
}

fn default_highway_to_style(highway_type: &str) -> RoadStyle {
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

/// Map `landuse=*` value to a surface block, honouring user overrides.
pub fn landuse_to_block(landuse: &str, ov: Option<&BlockOverrides>) -> Block {
    if let Some(o) = ov
        && let Some(&b) = o.landuse.get(landuse)
    {
        return b;
    }
    default_landuse_to_block(landuse)
}

fn default_landuse_to_block(landuse: &str) -> Block {
    match landuse {
        "forest" | "wood" => Block::OakLog,
        "grass" | "meadow" | "park" | "recreation_ground" | "village_green" => Block::GrassBlock,
        "farmland" | "farmyard" => Block::Dirt,
        "beach" | "sand" => Block::Sand,
        "reservoir" | "water" | "basin" => Block::Water,
        _ => Block::GrassBlock,
    }
}

/// Block for `natural=*` features, honouring user overrides.
pub fn natural_to_block(natural: &str, ov: Option<&BlockOverrides>) -> Block {
    if let Some(o) = ov
        && let Some(&b) = o.natural.get(natural)
    {
        return b;
    }
    default_natural_to_block(natural)
}

fn default_natural_to_block(natural: &str) -> Block {
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

/// Map a surface block to the nearest Java Edition biome ID string.
pub fn surface_to_java_biome(block: Block) -> &'static str {
    match block {
        Block::Water => "minecraft:river",
        Block::OakLog | Block::OakLeaves => "minecraft:forest",
        Block::BirchLog | Block::BirchLeaves => "minecraft:birch_forest",
        Block::Sand => "minecraft:beach",
        Block::Stone => "minecraft:windswept_hills",
        Block::Snow | Block::SnowLayer => "minecraft:snowy_plains",
        Block::Ice => "minecraft:frozen_river",
        _ => "minecraft:plains",
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
pub fn waterway_to_style(waterway_type: &str, tags: &TagMap, scale: f64) -> WaterwayStyle {
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

/// Choose a building wall block based on `building:material`, honouring user
/// overrides keyed by the material value.
pub fn building_block(tags: &TagMap, ov: Option<&BlockOverrides>) -> Block {
    if let Some(material) = tags.get("building:material")
        && let Some(o) = ov
        && let Some(&b) = o.building.get(material)
    {
        return b;
    }
    default_building_block(tags)
}

fn default_building_block(tags: &TagMap) -> Block {
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
    use crate::osm::TagMap;

    #[test]
    fn building_block_brick() {
        let mut tags = TagMap::new();
        tags.insert("building:material".into(), "brick".to_string());
        assert_eq!(building_block(&tags, None), Block::Brick);
    }

    #[test]
    fn building_block_default() {
        let tags = TagMap::new();
        assert_eq!(building_block(&tags, None), Block::StoneBrick);
    }

    #[test]
    fn building_block_wood() {
        let mut tags = TagMap::new();
        tags.insert("building:material".into(), "wood".to_string());
        assert_eq!(building_block(&tags, None), Block::OakPlanks);
    }

    // ── override behavior tests ─────────────────────────────────────────

    #[test]
    fn building_block_override_wins() {
        let mut ov = BlockOverrides::default();
        ov.building.insert("brick".to_string(), Block::OakPlanks);
        let mut tags = TagMap::new();
        tags.insert("building:material".into(), "brick".to_string());
        assert_eq!(building_block(&tags, Some(&ov)), Block::OakPlanks);
    }

    #[test]
    fn building_block_override_for_unknown_material_adds_mapping() {
        let mut ov = BlockOverrides::default();
        ov.building.insert("glass".to_string(), Block::GlassPane);
        let mut tags = TagMap::new();
        tags.insert("building:material".into(), "glass".to_string());
        // "glass" has no built-in mapping; without the override it would fall
        // back to StoneBrick. With it, it returns the override.
        assert_eq!(building_block(&tags, None), Block::StoneBrick);
        assert_eq!(building_block(&tags, Some(&ov)), Block::GlassPane);
    }

    #[test]
    fn landuse_override_and_default() {
        let mut ov = BlockOverrides::default();
        ov.landuse.insert("farmland".to_string(), Block::Sand);
        assert_eq!(landuse_to_block("farmland", None), Block::Dirt); // default
        assert_eq!(landuse_to_block("farmland", Some(&ov)), Block::Sand); // override
        assert_eq!(landuse_to_block("forest", Some(&ov)), Block::OakLog); // untouched default
    }

    #[test]
    fn natural_override_and_default() {
        let mut ov = BlockOverrides::default();
        ov.natural.insert("wood".to_string(), Block::BirchLog);
        assert_eq!(natural_to_block("wood", None), Block::OakLog); // default
        assert_eq!(natural_to_block("wood", Some(&ov)), Block::BirchLog); // override
    }

    #[test]
    fn highway_override_changes_surface_only() {
        let mut ov = BlockOverrides::default();
        ov.highway
            .insert("motorway".to_string(), Block::SmoothStoneSlab);
        let default_style = highway_to_style("motorway", None);
        let overridden = highway_to_style("motorway", Some(&ov));
        assert_eq!(overridden.surface, Block::SmoothStoneSlab); // surface replaced
        assert_eq!(overridden.half_width, default_style.half_width); // width preserved
        assert_eq!(overridden.sidewalk, default_style.sidewalk); // sidewalk preserved
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
        let tags = TagMap::new();
        let style = waterway_to_style("river", &tags, 1.0);
        assert_eq!(style.half_width, 3);
        assert_eq!(style.depth, 4);
    }

    #[test]
    fn waterway_style_canal() {
        let tags = TagMap::new();
        let style = waterway_to_style("canal", &tags, 1.0);
        assert_eq!(style.half_width, 2);
        assert_eq!(style.depth, 3);
    }

    #[test]
    fn waterway_style_stream() {
        let tags = TagMap::new();
        let style = waterway_to_style("stream", &tags, 1.0);
        assert_eq!(style.half_width, 1);
        assert_eq!(style.depth, 2);
    }

    #[test]
    fn waterway_style_ditch() {
        let tags = TagMap::new();
        let style = waterway_to_style("ditch", &tags, 1.0);
        assert_eq!(style.half_width, 0);
        assert_eq!(style.depth, 1);
    }

    #[test]
    fn waterway_style_drain() {
        let tags = TagMap::new();
        let style = waterway_to_style("drain", &tags, 1.0);
        assert_eq!(style.half_width, 0);
        assert_eq!(style.depth, 1);
    }

    #[test]
    fn waterway_style_default_fallback() {
        let tags = TagMap::new();
        let style = waterway_to_style("unknown_type", &tags, 1.0);
        assert_eq!(style.half_width, 1);
        assert_eq!(style.depth, 2);
    }

    #[test]
    fn waterway_style_width_tag_override() {
        let mut tags = TagMap::new();
        tags.insert("width".into(), "10.0".to_string());
        let style = waterway_to_style("stream", &tags, 1.0);
        assert_eq!(style.half_width, 5);
    }

    #[test]
    fn waterway_style_depth_tag_override() {
        let mut tags = TagMap::new();
        tags.insert("depth".into(), "6.0".to_string());
        let style = waterway_to_style("stream", &tags, 1.0);
        assert_eq!(style.depth, 6);
    }

    #[test]
    fn waterway_style_non_numeric_tags_ignored() {
        let mut tags = TagMap::new();
        tags.insert("width".into(), "narrow".to_string());
        tags.insert("depth".into(), "shallow".to_string());
        let style = waterway_to_style("river", &tags, 1.0);
        assert_eq!(style.half_width, 3);
        assert_eq!(style.depth, 4);
    }

    #[test]
    fn waterway_style_width_clamped() {
        let mut tags = TagMap::new();
        tags.insert("width".into(), "200.0".to_string());
        let style = waterway_to_style("river", &tags, 1.0);
        assert_eq!(style.half_width, 8);
    }

    #[test]
    fn waterway_style_depth_clamped_min() {
        let mut tags = TagMap::new();
        tags.insert("depth".into(), "0.0".to_string());
        let style = waterway_to_style("river", &tags, 1.0);
        assert_eq!(style.depth, 1);
    }

    #[test]
    fn waterway_style_scale_applied() {
        let mut tags = TagMap::new();
        tags.insert("width".into(), "4.0".to_string());
        let style = waterway_to_style("stream", &tags, 2.0);
        assert_eq!(style.half_width, 1);
    }

    #[test]
    fn waterway_style_depth_clamped_max() {
        let mut tags = TagMap::new();
        tags.insert("depth".into(), "100.0".to_string()); // far above max → clamped to 6
        let style = waterway_to_style("stream", &tags, 1.0);
        assert_eq!(style.depth, 6);
    }

    #[test]
    fn waterway_style_scale_applied_to_depth() {
        let mut tags = TagMap::new();
        tags.insert("depth".into(), "4.0".to_string()); // 4m at scale 2.0 → 2 blocks deep
        let style = waterway_to_style("stream", &tags, 2.0);
        assert_eq!(style.depth, 2);
    }

    // ── java_name tests ─────────────────────────────────────────────────────

    #[test]
    fn java_name_sign() {
        assert_eq!(Block::OakSign.java_name(), "minecraft:oak_sign");
        assert_eq!(Block::CherrySign.java_name(), "minecraft:cherry_sign");
    }

    #[test]
    fn java_name_brick() {
        assert_eq!(Block::Brick.java_name(), "minecraft:bricks");
    }

    #[test]
    fn java_name_poppy() {
        assert_eq!(Block::Poppy.java_name(), "minecraft:poppy");
    }

    #[test]
    fn java_name_stone_slab() {
        assert_eq!(Block::StoneSlab.java_name(), "minecraft:stone_slab");
    }

    #[test]
    fn java_name_tallgrass() {
        assert_eq!(Block::TallGrass.java_name(), "minecraft:tall_grass");
    }

    #[test]
    fn java_name_fern() {
        assert_eq!(Block::Fern.java_name(), "minecraft:fern");
    }

    #[test]
    fn java_name_stone_brick_wall() {
        assert_eq!(
            Block::StoneBrickWall.java_name(),
            "minecraft:stone_brick_wall"
        );
    }

    #[test]
    fn java_name_shared_blocks_unchanged() {
        // Blocks that share the same name between Bedrock and Java
        assert_eq!(Block::Stone.java_name(), "minecraft:stone");
        assert_eq!(Block::Bedrock.java_name(), "minecraft:bedrock");
        assert_eq!(Block::Water.java_name(), "minecraft:water");
        assert_eq!(Block::OakLog.java_name(), "minecraft:oak_log");
        assert_eq!(Block::OakLeaves.java_name(), "minecraft:oak_leaves");
        assert_eq!(Block::Cobblestone.java_name(), "minecraft:cobblestone");
        assert_eq!(Block::Snow.java_name(), "minecraft:snow");
        assert_eq!(Block::Ice.java_name(), "minecraft:ice");
    }

    // ── java_block_states tests ─────────────────────────────────────────────

    #[test]
    fn java_block_states_sign_has_rotation() {
        let states = Block::OakSign.java_block_states();
        assert_eq!(states, vec![("rotation", "0")]);

        let cherry_states = Block::CherrySign.java_block_states();
        assert_eq!(cherry_states, vec![("rotation", "0")]);
    }

    #[test]
    fn java_block_states_slab_has_half() {
        let states = Block::OakSlab.java_block_states();
        assert_eq!(states, vec![("type", "bottom")]);

        let bs_states = Block::PolishedBlackstoneSlab.java_block_states();
        assert_eq!(bs_states, vec![("type", "bottom")]);

        let smooth_states = Block::SmoothStoneSlab.java_block_states();
        assert_eq!(smooth_states, vec![("type", "bottom")]);

        let andesite_states = Block::AndesiteSlab.java_block_states();
        assert_eq!(andesite_states, vec![("type", "bottom")]);
    }

    #[test]
    fn java_block_states_stairs_has_facing() {
        let states = Block::OakStairs.java_block_states();
        assert_eq!(
            states,
            vec![
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
            ]
        );

        let sb_states = Block::StoneBrickStairs.java_block_states();
        assert_eq!(
            sb_states,
            vec![
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
            ]
        );
    }

    #[test]
    fn java_block_states_poppy_has_no_states() {
        assert_eq!(Block::Poppy.java_block_states(), vec![]);
    }

    #[test]
    fn java_block_states_log_has_axis() {
        assert_eq!(Block::BirchLog.java_block_states(), vec![("axis", "y")]);
    }

    // ── surface_to_java_biome tests ─────────────────────────────────────────

    #[test]
    fn java_biome_water() {
        assert_eq!(surface_to_java_biome(Block::Water), "minecraft:river");
    }

    #[test]
    fn java_biome_forest() {
        assert_eq!(surface_to_java_biome(Block::OakLog), "minecraft:forest");
        assert_eq!(surface_to_java_biome(Block::OakLeaves), "minecraft:forest");
    }

    #[test]
    fn java_biome_birch() {
        assert_eq!(
            surface_to_java_biome(Block::BirchLog),
            "minecraft:birch_forest"
        );
        assert_eq!(
            surface_to_java_biome(Block::BirchLeaves),
            "minecraft:birch_forest"
        );
    }

    #[test]
    fn java_biome_beach() {
        assert_eq!(surface_to_java_biome(Block::Sand), "minecraft:beach");
    }

    #[test]
    fn java_biome_mountains() {
        assert_eq!(
            surface_to_java_biome(Block::Stone),
            "minecraft:windswept_hills"
        );
    }

    #[test]
    fn java_biome_snow() {
        assert_eq!(surface_to_java_biome(Block::Snow), "minecraft:snowy_plains");
        assert_eq!(
            surface_to_java_biome(Block::SnowLayer),
            "minecraft:snowy_plains"
        );
    }

    #[test]
    fn java_biome_plains_default() {
        assert_eq!(surface_to_java_biome(Block::GrassBlock), "minecraft:plains");
        assert_eq!(surface_to_java_biome(Block::Dirt), "minecraft:plains");
        assert_eq!(surface_to_java_biome(Block::Concrete), "minecraft:plains");
        assert_eq!(surface_to_java_biome(Block::StoneBrick), "minecraft:plains");
    }

    // ── Block::from_name tests ──────────────────────────────────────────

    /// The authoritative list of (variant, name) pairs. Adding a new Block
    /// variant requires adding it here AND to `Block::from_name`. This test
    /// enforces that every listed variant round-trips through its name.
    const ALL_BLOCK_VARIANTS: &[(Block, &str)] = &[
        (Block::Air, "Air"),
        (Block::Bedrock, "Bedrock"),
        (Block::Stone, "Stone"),
        (Block::Dirt, "Dirt"),
        (Block::GrassBlock, "GrassBlock"),
        (Block::Water, "Water"),
        (Block::Sand, "Sand"),
        (Block::Gravel, "Gravel"),
        (Block::OakLog, "OakLog"),
        (Block::OakLeaves, "OakLeaves"),
        (Block::StoneBrick, "StoneBrick"),
        (Block::Concrete, "Concrete"),
        (Block::Cobblestone, "Cobblestone"),
        (Block::BlackConcrete, "BlackConcrete"),
        (Block::GrayConcrete, "GrayConcrete"),
        (Block::StoneSlab, "StoneSlab"),
        (Block::YellowConcrete, "YellowConcrete"),
        (Block::OakSign, "OakSign"),
        (Block::GlassPane, "GlassPane"),
        (Block::OakStairs, "OakStairs"),
        (Block::OakSlab, "OakSlab"),
        (Block::OakFence, "OakFence"),
        (Block::CobblestoneWall, "CobblestoneWall"),
        (Block::Brick, "Brick"),
        (Block::Sandstone, "Sandstone"),
        (Block::OakPlanks, "OakPlanks"),
        (Block::SprucePlanks, "SprucePlanks"),
        (Block::WhiteConcrete, "WhiteConcrete"),
        (Block::StoneBrickStairs, "StoneBrickStairs"),
        (Block::Rail, "Rail"),
        (Block::TallGrass, "TallGrass"),
        (Block::Fern, "Fern"),
        (Block::Poppy, "Poppy"),
        (Block::Torch, "Torch"),
        (Block::Lantern, "Lantern"),
        (Block::StoneBrickWall, "StoneBrickWall"),
        (Block::BirchLog, "BirchLog"),
        (Block::BirchLeaves, "BirchLeaves"),
        (Block::PolishedBlackstoneSlab, "PolishedBlackstoneSlab"),
        (Block::SmoothStoneSlab, "SmoothStoneSlab"),
        (Block::AndesiteSlab, "AndesiteSlab"),
        (Block::CherrySign, "CherrySign"),
        (Block::Snow, "Snow"),
        (Block::SnowLayer, "SnowLayer"),
        (Block::Ice, "Ice"),
        (Block::CherryHangingSign, "CherryHangingSign"),
        (Block::Dispenser, "Dispenser"),
        (Block::BrewingStand, "BrewingStand"),
        (Block::Bookshelf, "Bookshelf"),
        (Block::Cauldron, "Cauldron"),
        (Block::Bed, "Bed"),
        (Block::Furnace, "Furnace"),
        (Block::Barrel, "Barrel"),
        (Block::Bell, "Bell"),
        (Block::Campfire, "Campfire"),
        (Block::HayBale, "HayBale"),
    ];

    #[test]
    fn from_name_round_trips_all_variants() {
        assert_eq!(ALL_BLOCK_VARIANTS.len(), 56, "expected 56 Block variants");
        for &(block, name) in ALL_BLOCK_VARIANTS {
            assert_eq!(
                Block::from_name(name),
                Some(block),
                "from_name({name:?}) should return {block:?}"
            );
        }
    }

    #[test]
    fn from_name_rejects_unknown() {
        assert_eq!(Block::from_name("NotABlock"), None);
        assert_eq!(Block::from_name("oak_log"), None); // minecraft-id form is NOT accepted
        assert_eq!(Block::from_name("oaklog"), None); // case must match exactly
        assert_eq!(Block::from_name(""), None);
    }
}
