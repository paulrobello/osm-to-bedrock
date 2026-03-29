//! Sign text formatting and placement helpers.
//!
//! Provides functions for formatting street-name and POI signs, computing the
//! nearest road direction from a point, and converting direction vectors to
//! Bedrock sign-rotation values.

use crate::osm::OsmWay;

/// Format a street name for a sign: split into up to 4 lines of ~15 chars.
pub fn format_sign_text(name: &str) -> String {
    let max_line_len = 15;
    let max_lines = 4;
    let mut lines: Vec<String> = Vec::new();
    let mut current_line = String::new();

    for word in name.split_whitespace() {
        // `char_count` avoids byte-indexing into a potentially multibyte string.
        let word_char_len = word.chars().count();
        if current_line.is_empty() {
            // If a single word exceeds max_line_len, truncate at a char boundary.
            if word_char_len > max_line_len {
                current_line = word.chars().take(max_line_len).collect();
            } else {
                current_line = word.to_string();
            }
        } else if current_line.chars().count() + 1 + word_char_len <= max_line_len {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current_line));
            if lines.len() >= max_lines {
                break;
            }
            if word_char_len > max_line_len {
                current_line = word.chars().take(max_line_len).collect();
            } else {
                current_line = word.to_string();
            }
        }
    }

    if !current_line.is_empty() && lines.len() < max_lines {
        lines.push(current_line);
    }

    lines.join("\n")
}

/// Format a POI sign: type label on line 1, then the place name (if any) on subsequent lines.
pub fn format_poi_sign(name: &str, poi_type: &str) -> String {
    let label: String = match poi_type {
        "restaurant" | "cafe" | "fast_food" | "bar" | "pub" | "biergarten" => {
            "[Food/Drink]".to_string()
        }
        "hospital" | "clinic" | "doctors" | "dentist" => "[Medical]".to_string(),
        "school" | "university" | "college" | "kindergarten" => "[Education]".to_string(),
        "bank" | "atm" => "[Finance]".to_string(),
        "pharmacy" => "[Pharmacy]".to_string(),
        "fuel" => "[Gas Station]".to_string(),
        "post_office" => "[Post Office]".to_string(),
        "place_of_worship" => "[Worship]".to_string(),
        "library" => "[Library]".to_string(),
        "police" => "[Police]".to_string(),
        "fire_station" => "[Fire Stn]".to_string(),
        "hotel" | "hostel" | "motel" | "guest_house" => "[Lodging]".to_string(),
        "museum" | "gallery" => "[Museum]".to_string(),
        "attraction" | "viewpoint" => "[Attraction]".to_string(),
        "park" | "playground" | "sports_centre" | "stadium" => "[Leisure]".to_string(),
        "parking" => "[Parking]".to_string(),
        "toilets" => "[Toilets]".to_string(),
        "supermarket" | "convenience" => "[Shop]".to_string(),
        _ => {
            // Title-case the raw type, replacing underscores with spaces
            let words: String = poi_type
                .split('_')
                .map(|w| {
                    let mut chars = w.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("[{}]", words)
        }
    };
    if name.is_empty() {
        label
    } else {
        let name_text = format_sign_text(name);
        // Take only first 3 name lines to stay within 4-line sign limit
        let name_lines: Vec<&str> = name_text.lines().take(3).collect();
        format!("{}\n{}", label, name_lines.join("\n"))
    }
}

/// Direction vector (f64 dx, dz) from `(sx, sz)` to the nearest point on any
/// highway way, within `max_dist` blocks.  Returns `None` if no road is found.
pub fn nearest_road_vector(
    sx: i32,
    sz: i32,
    highways: &[usize],
    resolved_ways: &[(&OsmWay, Vec<(i32, i32)>)],
    max_dist: i32,
) -> Option<(f64, f64)> {
    let max_dist_sq = (max_dist as i64) * (max_dist as i64);
    let mut best_dist_sq = max_dist_sq;
    let mut best: Option<(f64, f64)> = None;
    for &wi in highways {
        let (_, pts) = &resolved_ways[wi];
        for &(rx, rz) in pts {
            let dx = (rx - sx) as i64;
            let dz = (rz - sz) as i64;
            let d_sq = dx * dx + dz * dz;
            if d_sq > 0 && d_sq < best_dist_sq {
                best_dist_sq = d_sq;
                best = Some((dx as f64, dz as f64));
            }
        }
    }
    best
}

/// Convert a direction vector `(dx, dz)` to a Bedrock standing-sign rotation
/// value (0–15).  The sign will face toward `(dx, dz)`.
///
/// Bedrock convention: 0 = south (+Z), 4 = west (−X), 8 = north (−Z),
/// 12 = east (+X), values increase clockwise.
pub fn vec_to_sign_dir(dx: f64, dz: f64) -> i32 {
    let angle = dz.atan2(dx);
    // Bedrock "south-clockwise" bearing: south-bearing = atan2_deg + 270° (mod 360°).
    // In radians: (angle + 3π/2) mod 2π, then scale to 0-16.
    let dir_f = (angle + 3.0 * std::f64::consts::PI / 2.0).rem_euclid(2.0 * std::f64::consts::PI)
        / (2.0 * std::f64::consts::PI)
        * 16.0;
    dir_f.round() as i32 % 16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_to_sign_dir_cardinals() {
        // 0=south(+Z), 4=west(-X), 8=north(-Z), 12=east(+X)
        assert_eq!(vec_to_sign_dir(0.0, 1.0), 0, "south (+Z) → 0");
        assert_eq!(vec_to_sign_dir(-1.0, 0.0), 4, "west (-X) → 4");
        assert_eq!(vec_to_sign_dir(0.0, -1.0), 8, "north (-Z) → 8");
        assert_eq!(vec_to_sign_dir(1.0, 0.0), 12, "east (+X) → 12");
    }

    #[test]
    fn format_poi_sign_known_type() {
        let text = format_poi_sign("", "restaurant");
        assert_eq!(text, "[Food/Drink]");
    }

    #[test]
    fn format_poi_sign_with_name() {
        let text = format_poi_sign("The Pub", "restaurant");
        assert!(text.starts_with("[Food/Drink]"));
        assert!(text.contains("The Pub"));
    }

    #[test]
    fn format_poi_sign_unknown_type() {
        let text = format_poi_sign("", "unknown_poi");
        assert_eq!(text, "[Unknown Poi]");
    }
}
