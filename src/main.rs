//! OSM to Minecraft Bedrock Edition world converter — binary entry point.
//!
//! All CLI parsing, dispatch, and subcommand helpers live in the library's
//! [`osm_to_bedrock::cli`] module; the binary is a one-line shim so the CLI
//! types and dispatch can be unit-tested like the rest of the crate.
//!
//! ## Usage
//! ```text
//! osm-to-bedrock convert --input map.osm.pbf --output MyWorld/
//! osm-to-bedrock convert --input map.osm.pbf --output MyWorld/ --scale 2.0 --sea-level 62
//! osm-to-bedrock serve --port 3002 --host 127.0.0.1
//! ```

fn main() -> anyhow::Result<()> {
    osm_to_bedrock::cli::main()
}
