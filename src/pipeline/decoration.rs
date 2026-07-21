//! POI / tree decoration placement.
//!
//! Per-feature decoration blocks (furniture for shops, beds for hotels,
//! species-varied trees) are rendered on top of the surface column. The
//! helpers here are called from [`super::render::render_osm_features`] and
//! are `pub(super)` so they remain invisible outside the [`crate::pipeline`]
//! module.

use crate::osm::TagMap;

use crate::blocks::Block;
use crate::world::WorldWriter;

use super::util::coord_hash;

/// Extract the best POI type label from a tag set.
///
/// Tries standard OSM keys first (`amenity`, `shop`, `tourism`, `leisure`,
/// `historic`), then falls back to any non-metadata tag value that could serve
/// as a meaningful label.
pub(super) fn resolve_poi_type(tags: &TagMap) -> &str {
    // Standard OSM POI keys
    const POI_KEYS: &[&str] = &["amenity", "shop", "tourism", "leisure", "historic"];
    for key in POI_KEYS {
        if let Some(v) = tags.get(*key) {
            return v.as_str();
        }
    }
    // Fallback: pick the first tag whose key isn't a metadata/structural field
    const SKIP_KEYS: &[&str] = &[
        "name",
        "building",
        "building:height",
        "building:levels",
        "highway",
        "surface",
        "bridge",
        "tunnel",
        "railway",
        "waterway",
        "natural",
        "landuse",
        "addr:housenumber",
        "addr:street",
        "barrier",
    ];
    for (k, v) in tags {
        let key: &str = k;
        if !SKIP_KEYS.contains(&key) && !v.is_empty() {
            return v.as_str();
        }
    }
    "poi"
}

/// Place a decorative block structure at a POI location.
pub(super) fn place_poi_decoration(
    world: &mut dyn WorldWriter,
    x: i32,
    sy: i32,
    z: i32,
    poi_type: &str,
) {
    match poi_type {
        // Coffee specifically — brewing stand
        "coffee_shop" => {
            world.set_block(x, sy + 1, z, Block::BrewingStand);
        }
        // Food & Drink — furnace (kitchen)
        "restaurant"
        | "cafe"
        | "fast_food"
        | "bar"
        | "pub"
        | "biergarten"
        | "food_court"
        | "mexican_restaurant"
        | "pizza_restaurant"
        | "fast_food_restaurant"
        | "breakfast_and_brunch_restaurant"
        | "barbecue_restaurant" => {
            world.set_block(x, sy + 1, z, Block::Furnace);
        }
        // Lodging — bed
        "hotel" | "motel" | "hostel" | "guest_house" => {
            world.set_block(x, sy + 1, z, Block::Bed);
        }
        // Education — bookshelf
        "school" | "university" | "college" | "kindergarten" | "library" | "elementary_school" => {
            world.set_block(x, sy + 1, z, Block::Bookshelf);
        }
        // Medical — white concrete + red concrete cross (2 blocks tall)
        "hospital" | "clinic" | "doctors" | "dentist" | "pharmacy" | "medical_center"
        | "doctor" | "optometrist" | "pediatric_dentist" => {
            world.set_block(x, sy + 1, z, Block::WhiteConcrete);
            world.set_block(x, sy + 2, z, Block::WhiteConcrete);
        }
        // Worship — bell
        "place_of_worship" | "church_cathedral" => {
            world.set_block(x, sy + 1, z, Block::OakFence);
            world.set_block(x, sy + 2, z, Block::Bell);
        }
        // Post / mail — dispenser on fence (mailbox)
        "post_office" => {
            world.set_block(x, sy + 1, z, Block::OakFence);
            world.set_block(x, sy + 2, z, Block::Dispenser);
        }
        // Fire station — campfire + lantern
        "fire_station" => {
            world.set_block(x, sy + 1, z, Block::Campfire);
            world.set_block(x, sy + 2, z, Block::Lantern);
        }
        // Farm — hay bale
        "farm" => {
            world.set_block(x, sy + 1, z, Block::HayBale);
        }
        // Gas station — dispenser (fuel pump)
        "gas_station" | "fuel" => {
            world.set_block(x, sy + 1, z, Block::Dispenser);
        }
        // Parking — iron bars
        "parking" => {
            world.set_block(x, sy + 1, z, Block::OakFence);
        }
        // Banks / ATM — barrel (vault)
        "bank" | "atm" | "atms" | "banks" | "bank_credit_union" | "financial_service" => {
            world.set_block(x, sy + 1, z, Block::Barrel);
        }
        // Shops / stores — barrel
        "supermarket" | "convenience" | "convenience_store" | "grocery_store"
        | "department_store" | "mall" => {
            world.set_block(x, sy + 1, z, Block::Barrel);
        }
        // Default: lantern on fence post (street furniture)
        _ => {
            world.set_block(x, sy + 1, z, Block::OakFence);
            world.set_block(x, sy + 2, z, Block::Lantern);
        }
    }
}

/// Place a tree at an exact position (from individual tree node data).
pub(super) fn place_tree(world: &mut dyn WorldWriter, x: i32, z: i32, sy: i32) {
    let species = coord_hash(x, z) % 5;
    let (log_block, leaf_block) = match species {
        0..=2 => (Block::OakLog, Block::OakLeaves),
        3 => (Block::BirchLog, Block::BirchLeaves),
        _ => (Block::OakLog, Block::OakLeaves),
    };
    let trunk_height: i32 = if species == 4 { 6 } else { 4 };
    let canopy_radius: i32 = if species == 4 { 3 } else { 2 };

    for dy in 1..=trunk_height {
        world.set_block(x, sy + dy, z, log_block);
    }
    for lx in -canopy_radius..=canopy_radius {
        for lz in -canopy_radius..=canopy_radius {
            if lx.abs() == canopy_radius && lz.abs() == canopy_radius {
                continue; // round the corners
            }
            world.set_block(x + lx, sy + trunk_height + 1, z + lz, leaf_block);
            if canopy_radius > 2 {
                world.set_block(x + lx, sy + trunk_height, z + lz, leaf_block);
            }
        }
    }
    // Top cap
    world.set_block(x, sy + trunk_height + 2, z, leaf_block);
}

/// Place a tree or undergrowth at `(x, sy, z)` if the block is OakLog (forest)
/// and the coordinate hash selects this position.
pub(super) fn maybe_place_tree(world: &mut dyn WorldWriter, x: i32, z: i32, sy: i32, block: Block) {
    if block == Block::OakLog && coord_hash(x, z).is_multiple_of(7) {
        let species = coord_hash(x, z) % 5;
        let (log_block, leaf_block) = match species {
            0..=2 => (Block::OakLog, Block::OakLeaves), // 60% oak
            3 => (Block::BirchLog, Block::BirchLeaves), // 20% birch
            _ => (Block::OakLog, Block::OakLeaves),     // 20% tall oak variant
        };
        let trunk_height: i32 = if species == 4 { 6 } else { 4 };
        let canopy_radius: i32 = if species == 4 { 1 } else { 2 };

        for dy in 1..=trunk_height {
            world.set_block(x, sy + dy, z, log_block);
        }
        for lx in -canopy_radius..=canopy_radius {
            for lz in -canopy_radius..=canopy_radius {
                world.set_block(x + lx, sy + trunk_height + 1, z + lz, leaf_block);
            }
        }
        if coord_hash(x + 2, z).is_multiple_of(15) {
            world.set_block(x, sy + trunk_height + 2, z, Block::Torch);
        }
    } else if block == Block::OakLog && coord_hash(x + 1, z + 1).is_multiple_of(3) {
        // Undergrowth between trees
        let plant_roll = coord_hash(x, z + 1) % 10;
        let plant = if plant_roll < 5 {
            Block::TallGrass
        } else if plant_roll < 8 {
            Block::Fern
        } else {
            Block::Poppy
        };
        world.set_block(x, sy + 1, z, plant);
    }
}
