//! Downloads the in-game map art for every uiMapID the website knows
//! (`frontend/data/maps.json`) and writes one stitched image per map to
//! `frontend/maps/<uiMapID>.jpg`. The site then shows the real zone map
//! under the player's position instead of the schematic.
//!
//! Source: wago.tools, which mirrors the game's DB2 tables and files. The
//! tables chain like this:
//!   UiMapXMapArt (UiMapID -> UiMapArtID)
//!   UiMapArt (-> UiMapArtStyleID)
//!   UiMapArtStyleLayer (layer 0 size and tile size for that style)
//!   UiMapArtTile (row, column, FileDataID of each BLP tile)
//!   WorldMapOverlay + WorldMapOverlayTile (the explored-area pieces, drawn
//!   on top so the map looks fully explored, as in game once discovered)
//!
//! The art belongs to Blizzard; the images are git-ignored and only used
//! on your own non-commercial site.

mod blp;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_BUILD: &str = "1.60.1.70790";
const USAGE: &str = "\
mapfetch: download the game's map art for the website

USAGE: mapfetch [--maps frontend/data/maps.json] [--out frontend/maps]
                [--build 1.60.1.70790] [--only 1429,1438] [--force] [--quality 85]
                [--unexplored]

Skips maps that already have an image unless --force is given.
--unexplored leaves out the explored-area overlays (the bare base map).";

struct Args {
    maps: PathBuf,
    out: PathBuf,
    build: String,
    only: Option<Vec<u32>>,
    force: bool,
    quality: u8,
    unexplored: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        maps: "frontend/data/maps.json".into(),
        out: "frontend/maps".into(),
        build: DEFAULT_BUILD.into(),
        only: None,
        force: false,
        quality: 85,
        unexplored: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--maps" => a.maps = val()?.into(),
            "--out" => a.out = val()?.into(),
            "--build" => a.build = val()?,
            "--only" => {
                let ids = val()?.split(',').map(|s| s.trim().parse::<u32>().map_err(|_| format!("bad map id '{s}'"))).collect::<Result<_, _>>()?;
                a.only = Some(ids);
            }
            "--quality" => a.quality = val()?.parse().map_err(|_| "--quality takes 1-100")?,
            "--force" => a.force = true,
            "--unexplored" => a.unexplored = true,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument '{other}'\n\n{USAGE}")),
        }
    }
    Ok(a)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

struct Http {
    agent: ureq::Agent,
}

impl Http {
    fn new() -> Http {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("wowtracker-mapfetch/", env!("CARGO_PKG_VERSION")))
            .build();
        Http { agent }
    }

    fn bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut last = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(2 * attempt));
            }
            match self.agent.get(url).call() {
                Ok(resp) => {
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf).map_err(|e| format!("{url}: {e}"))?;
                    return Ok(buf);
                }
                Err(ureq::Error::Status(404, _)) => return Err(format!("{url}: not found")),
                Err(e) => last = format!("{url}: {e}"),
            }
        }
        Err(last)
    }

    fn table(&self, name: &str, build: &str) -> Result<Table, String> {
        let body = self.bytes(&format!("https://wago.tools/db2/{name}/csv?build={build}"))?;
        Table::parse(&String::from_utf8_lossy(&body)).map_err(|e| format!("{name}: {e}"))
    }
}

/// A DB2 table as CSV. These tables hold only numbers, so no quoting.
struct Table {
    cols: HashMap<String, usize>,
    rows: Vec<Vec<i64>>,
}

impl Table {
    fn parse(text: &str) -> Result<Table, String> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let head = lines.next().ok_or("empty CSV")?;
        let cols = head.split(',').enumerate().map(|(i, c)| (c.trim().to_string(), i)).collect();
        let rows = lines.map(|l| l.split(',').map(|v| v.trim().parse::<f64>().map(|f| f as i64).unwrap_or(0)).collect()).collect();
        Ok(Table { cols, rows })
    }

    fn col(&self, name: &str) -> Result<usize, String> {
        self.cols.get(name).copied().ok_or_else(|| format!("no column {name}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Layer {
    width: u32,
    height: u32,
    tile_w: u32,
    tile_h: u32,
}

#[derive(Debug, PartialEq)]
struct Tile {
    row: u32,
    col: u32,
    file: u32,
}

/// An explored-area piece: a texture of `width`×`height` pixels placed at
/// (`x`, `y`) on the base layer, cut into tiles of the layer's tile size.
#[derive(Debug, PartialEq)]
struct Overlay {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    tiles: Vec<Tile>,
}

#[derive(Debug, PartialEq)]
struct ArtPlan {
    layer: Layer,
    tiles: Vec<Tile>,
    overlays: Vec<Overlay>,
}

/// Explored-area overlays per UiMapArtID.
fn overlays(ov: &Table, ov_tiles: &Table) -> Result<HashMap<u32, Vec<Overlay>>, String> {
    let (o_id, o_art, o_w, o_h, o_x, o_y) =
        (ov.col("ID")?, ov.col("UiMapArtID")?, ov.col("TextureWidth")?, ov.col("TextureHeight")?, ov.col("OffsetX")?, ov.col("OffsetY")?);
    let (t_ov, t_layer, t_row, t_col, t_file) = (
        ov_tiles.col("WorldMapOverlayID")?,
        ov_tiles.col("LayerIndex")?,
        ov_tiles.col("RowIndex")?,
        ov_tiles.col("ColIndex")?,
        ov_tiles.col("FileDataID")?,
    );
    let mut tiles: HashMap<u32, Vec<Tile>> = HashMap::new();
    for r in ov_tiles.rows.iter().filter(|r| r[t_layer] == 0 && r[t_file] > 0) {
        tiles.entry(r[t_ov] as u32).or_default().push(Tile { row: r[t_row] as u32, col: r[t_col] as u32, file: r[t_file] as u32 });
    }
    let mut out: HashMap<u32, Vec<Overlay>> = HashMap::new();
    for r in &ov.rows {
        let Some(mut t) = tiles.remove(&(r[o_id] as u32)) else { continue };
        t.sort_by_key(|t| (t.row, t.col));
        out.entry(r[o_art] as u32).or_default().push(Overlay {
            x: r[o_x].max(0) as u32,
            y: r[o_y].max(0) as u32,
            width: r[o_w].max(0) as u32,
            height: r[o_h].max(0) as u32,
            tiles: t,
        });
    }
    Ok(out)
}

/// Works out the layer size and tiles for each uiMapID from the four tables.
fn plan(
    xmap: &Table,
    art: &Table,
    layers: &Table,
    tiles: &Table,
    mut overlays: HashMap<u32, Vec<Overlay>>,
) -> Result<BTreeMap<u32, ArtPlan>, String> {
    // uiMapID -> art ID, preferring phase 0.
    let (xm_map, xm_art, xm_phase) = (xmap.col("UiMapID")?, xmap.col("UiMapArtID")?, xmap.col("PhaseID")?);
    let mut map_art: HashMap<u32, (i64, u32)> = HashMap::new();
    for r in &xmap.rows {
        let (map, a, phase) = (r[xm_map] as u32, r[xm_art] as u32, r[xm_phase]);
        let better = map_art.get(&map).is_none_or(|(p, _)| *p != 0 && phase == 0);
        if better {
            map_art.insert(map, (phase, a));
        }
    }
    let (a_id, a_style) = (art.col("ID")?, art.col("UiMapArtStyleID")?);
    let art_style: HashMap<u32, u32> = art.rows.iter().map(|r| (r[a_id] as u32, r[a_style] as u32)).collect();

    let (l_style, l_idx) = (layers.col("UiMapArtStyleID")?, layers.col("LayerIndex")?);
    let (lw, lh, tw, th) = (layers.col("LayerWidth")?, layers.col("LayerHeight")?, layers.col("TileWidth")?, layers.col("TileHeight")?);
    let style_layer: HashMap<u32, Layer> = layers
        .rows
        .iter()
        .filter(|r| r[l_idx] == 0)
        .map(|r| (r[l_style] as u32, Layer { width: r[lw] as u32, height: r[lh] as u32, tile_w: r[tw] as u32, tile_h: r[th] as u32 }))
        .collect();

    let (t_art, t_layer, t_row, t_col, t_file) = (tiles.col("UiMapArtID")?, tiles.col("LayerIndex")?, tiles.col("RowIndex")?, tiles.col("ColIndex")?, tiles.col("FileDataID")?);
    let mut art_tiles: HashMap<u32, Vec<Tile>> = HashMap::new();
    for r in tiles.rows.iter().filter(|r| r[t_layer] == 0 && r[t_file] > 0) {
        art_tiles.entry(r[t_art] as u32).or_default().push(Tile { row: r[t_row] as u32, col: r[t_col] as u32, file: r[t_file] as u32 });
    }

    let mut out = BTreeMap::new();
    for (map, (_, a)) in map_art {
        let (Some(style), Some(mut t)) = (art_style.get(&a), art_tiles.remove(&a)) else { continue };
        let Some(&layer) = style_layer.get(style) else { continue };
        if layer.width == 0 || layer.tile_w == 0 {
            continue;
        }
        t.sort_by_key(|t| (t.row, t.col));
        out.insert(map, ArtPlan { layer, tiles: t, overlays: overlays.remove(&a).unwrap_or_default() });
    }
    Ok(out)
}

/// Pastes RGBA tiles onto one canvas and crops it to the layer size.
fn stitch(layer: Layer, tiles: &[(u32, u32, blp::Rgba)]) -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(layer.width, layer.height, image::Rgb([0, 0, 0]));
    for (row, col, t) in tiles {
        let (ox, oy) = (col * layer.tile_w, row * layer.tile_h);
        for y in 0..t.height {
            for x in 0..t.width {
                let (px, py) = (ox + x, oy + y);
                if px >= layer.width || py >= layer.height {
                    continue;
                }
                let i = ((y * t.width + x) * 4) as usize;
                img.put_pixel(px, py, image::Rgb([t.pixels[i], t.pixels[i + 1], t.pixels[i + 2]]));
            }
        }
    }
    img
}

/// Draws an overlay's tiles over the image with alpha blending. Each tile
/// covers `tile_w`×`tile_h` of the overlay (less at the right and bottom
/// edges); the BLP file can be larger, so only its top-left part is used.
fn paint_overlay(img: &mut image::RgbImage, layer: Layer, ov: &Overlay, tiles: &[(u32, u32, blp::Rgba)]) {
    for (row, col, t) in tiles {
        let (tx, ty) = (col * layer.tile_w, row * layer.tile_h);
        for y in 0..t.height.min(layer.tile_h) {
            for x in 0..t.width.min(layer.tile_w) {
                let (in_x, in_y) = (tx + x, ty + y);
                if in_x >= ov.width || in_y >= ov.height {
                    continue;
                }
                let (px, py) = (ov.x + in_x, ov.y + in_y);
                if px >= img.width() || py >= img.height() {
                    continue;
                }
                let i = ((y * t.width + x) * 4) as usize;
                let a = t.pixels[i + 3] as u32;
                if a == 0 {
                    continue;
                }
                let dst = img.get_pixel_mut(px, py);
                for c in 0..3 {
                    dst[c] = ((t.pixels[i + c] as u32 * a + dst[c] as u32 * (255 - a)) / 255) as u8;
                }
            }
        }
    }
}

fn wanted_maps(path: &Path) -> Result<BTreeMap<u32, String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let maps = v.get("maps").and_then(|m| m.as_object()).ok_or_else(|| format!("{}: no \"maps\" object", path.display()))?;
    Ok(maps
        .iter()
        .filter_map(|(id, m)| Some((id.parse().ok()?, m.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string())))
        .collect())
}

fn run(a: &Args) -> Result<(), String> {
    let mut wanted = wanted_maps(&a.maps)?;
    if let Some(only) = &a.only {
        wanted.retain(|id, _| only.contains(id));
    }
    std::fs::create_dir_all(&a.out).map_err(|e| format!("cannot create {}: {e}", a.out.display()))?;
    let exists = |id: u32| ["webp", "jpg", "png"].iter().any(|ext| a.out.join(format!("{id}.{ext}")).exists());
    if !a.force {
        wanted.retain(|id, _| !exists(*id));
    }
    if wanted.is_empty() {
        println!("All maps already have images (use --force to download again).");
        return Ok(());
    }

    let http = Http::new();
    println!("Reading map tables for build {} from wago.tools…", a.build);
    let ovs = if a.unexplored {
        HashMap::new()
    } else {
        overlays(&http.table("WorldMapOverlay", &a.build)?, &http.table("WorldMapOverlayTile", &a.build)?)?
    };
    let plans = plan(
        &http.table("UiMapXMapArt", &a.build)?,
        &http.table("UiMapArt", &a.build)?,
        &http.table("UiMapArtStyleLayer", &a.build)?,
        &http.table("UiMapArtTile", &a.build)?,
        ovs,
    )?;

    let (mut done, mut failed) = (0, 0);
    for (id, name) in &wanted {
        let Some(p) = plans.get(id) else {
            println!("  {id} {name}: no map art in this build, skipped");
            continue;
        };
        let n_ov: usize = p.overlays.iter().map(|o| o.tiles.len()).sum();
        print!("  {id} {name} ({}x{}, {} tiles, {} overlay tiles)… ", p.layer.width, p.layer.height, p.tiles.len(), n_ov);
        let fetch = |t: &Tile| -> Result<(u32, u32, blp::Rgba), String> {
            let bytes = http.bytes(&format!("https://wago.tools/api/casc/{}?version={}", t.file, a.build))?;
            Ok((t.row, t.col, blp::decode(&bytes).map_err(|e| format!("tile {}: {e}", t.file))?))
        };
        let result = (|| -> Result<(), String> {
            let decoded = p.tiles.iter().map(fetch).collect::<Result<Vec<_>, _>>()?;
            let mut img = stitch(p.layer, &decoded);
            for ov in &p.overlays {
                let tiles = ov.tiles.iter().map(fetch).collect::<Result<Vec<_>, _>>()?;
                paint_overlay(&mut img, p.layer, ov, &tiles);
            }
            let path = a.out.join(format!("{id}.jpg"));
            let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), a.quality);
            enc.encode_image(&img).map_err(|e| format!("{}: {e}", path.display()))
        })();
        match result {
            Ok(()) => {
                println!("ok");
                done += 1;
            }
            Err(e) => {
                println!("failed: {e}");
                failed += 1;
            }
        }
    }
    println!("Wrote {done} map image(s) to {}{}.", a.out.display(), if failed > 0 { format!(", {failed} failed") } else { String::new() });
    if failed > 0 {
        return Err(format!("{failed} map(s) failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(csv: &str) -> Table {
        Table::parse(csv).unwrap()
    }

    #[test]
    fn plans_from_tables() {
        let xmap = t("ID,PhaseID,UiMapArtID,UiMapID\n1,5,900,1429\n2,0,100,1429\n3,0,200,1438\n");
        let art = t("ID,HighlightFileDataID,HighlightAtlasID,UiMapArtStyleID\n100,0,0,1\n200,0,0,9\n900,0,0,1\n");
        let layers = t("ID,LayerIndex,LayerWidth,LayerHeight,TileWidth,TileHeight,MinScale,MaxScale,AdditionalZoomSteps,UiMapArtStyleID\n1,0,1002,668,256,256,1,2.14,2,1\n2,1,2004,1336,256,256,1,1,0,1\n");
        let tiles = t("ID,RowIndex,ColIndex,LayerIndex,FileDataID,UiMapArtID\n1,0,1,0,11,100\n2,0,0,0,10,100\n3,0,0,1,99,100\n4,0,0,0,20,200\n");
        let ov = t("ID,UiMapArtID,TextureWidth,TextureHeight,OffsetX,OffsetY,HitRectTop,HitRectBottom,HitRectLeft,HitRectRight,PlayerConditionID,Flags,AreaID_0,AreaID_1,AreaID_2,AreaID_3\n7,100,300,200,40,50,0,0,0,0,0,0,87,0,0,0\n");
        let ovt = t("ID,RowIndex,ColIndex,LayerIndex,FileDataID,WorldMapOverlayID\n1,0,1,0,71,7\n2,0,0,0,70,7\n");
        let p = plan(&xmap, &art, &layers, &tiles, overlays(&ov, &ovt).unwrap()).unwrap();
        let elwynn = &p[&1429];
        assert_eq!(elwynn.layer, Layer { width: 1002, height: 668, tile_w: 256, tile_h: 256 });
        assert_eq!(elwynn.tiles, vec![Tile { row: 0, col: 0, file: 10 }, Tile { row: 0, col: 1, file: 11 }], "phase 0, layer 0, sorted");
        assert!(!p.contains_key(&1438), "style 9 has no layer");
        assert_eq!(
            elwynn.overlays,
            vec![Overlay { x: 40, y: 50, width: 300, height: 200, tiles: vec![Tile { row: 0, col: 0, file: 70 }, Tile { row: 0, col: 1, file: 71 }] }]
        );
    }

    #[test]
    fn overlay_blends_and_clips() {
        let layer = Layer { width: 10, height: 10, tile_w: 2, tile_h: 2 };
        let mut img = image::RgbImage::from_pixel(10, 10, image::Rgb([0, 0, 0]));
        // A 3x2 overlay at (5,5): tile (0,0) full, tile (0,1) only 1 px wide.
        let ov = Overlay { x: 5, y: 5, width: 3, height: 2, tiles: vec![] };
        // BLP tiles are 4x4 (power of two); only the top-left 2x2 counts.
        let opaque = blp::Rgba { width: 4, height: 4, pixels: [200, 200, 200, 255].repeat(16) };
        let half = blp::Rgba { width: 4, height: 4, pixels: [200, 200, 200, 128].repeat(16) };
        paint_overlay(&mut img, layer, &ov, &[(0, 0, opaque), (0, 1, half)]);
        assert_eq!(img.get_pixel(5, 5)[0], 200);
        assert_eq!(img.get_pixel(6, 6)[0], 200);
        assert_eq!(img.get_pixel(7, 5)[0], 100, "half alpha blends");
        assert_eq!(img.get_pixel(8, 5)[0], 0, "past the overlay width");
        assert_eq!(img.get_pixel(5, 7)[0], 0, "past the overlay height");
    }

    #[test]
    fn stitches_and_crops() {
        let layer = Layer { width: 3, height: 3, tile_w: 2, tile_h: 2 };
        let solid = |v: u8| blp::Rgba { width: 2, height: 2, pixels: [v, v, v, 255].repeat(4) };
        let img = stitch(layer, &[(0, 0, solid(10)), (0, 1, solid(20)), (1, 0, solid(30)), (1, 1, solid(40))]);
        assert_eq!(img.dimensions(), (3, 3));
        assert_eq!(img.get_pixel(1, 1)[0], 10);
        assert_eq!(img.get_pixel(2, 0)[0], 20);
        assert_eq!(img.get_pixel(0, 2)[0], 30);
        assert_eq!(img.get_pixel(2, 2)[0], 40);
    }
}
