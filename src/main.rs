use actix_multipart::Multipart;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use futures_util::StreamExt;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap};
use std::io::Cursor;
use std::sync::Mutex;
use uuid::Uuid;

const WALL_WIDTH_PLATES: u32 = 27;
const WALL_HEIGHT_PLATES: u32 = 12;
const PLATE_SIZE: u32 = 30;
const TOTAL_WIDTH: u32 = WALL_WIDTH_PLATES * PLATE_SIZE;
const TOTAL_HEIGHT: u32 = WALL_HEIGHT_PLATES * PLATE_SIZE;

const APPROVED_COLORS: &[(&str, u8, u8, u8)] = &[
    ("VIT", 255, 255, 255),
    ("GUL", 255, 255, 0),
    ("ORANGE", 255, 165, 0),
    ("RÖD", 255, 0, 0),
    ("LILA", 128, 0, 128),
    ("ROSA", 255, 192, 203),
    ("BLÅ", 0, 0, 255),
    ("LJUSGRÖN", 144, 238, 144),
    ("BRUN", 139, 69, 19),
    ("SVART", 0, 0, 0),
];

struct AppState {
    db: Mutex<Connection>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UploadedImage {
    pub id: String,
    pub filename: String,
    pub plate_x: u32,
    pub plate_y: u32,
    pub votes: u32,
    pub uploaded_at: String,
    pub pixel_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WallState {
    pub week: u32,
    pub current_target: Option<TargetCoordinate>,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TargetCoordinate {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VoteRequest {
    pub image_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoordinateVoteRequest {
    pub x: u32,
    pub y: u32,
}

fn color_distance(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f64 {
    let dr = (r1 as f64 - r2 as f64).powi(2);
    let dg = (g1 as f64 - g2 as f64).powi(2);
    let db = (b1 as f64 - b2 as f64).powi(2);
    (dr + dg + db).sqrt()
}

fn find_closest_approved_color(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let mut closest = (APPROVED_COLORS[0].1, APPROVED_COLORS[0].2, APPROVED_COLORS[0].3);
    let mut min_dist = f64::MAX;

    for (_, ar, ag, ab) in APPROVED_COLORS {
        let dist = color_distance(r, g, b, *ar, *ag, *ab);
        if dist < min_dist {
            min_dist = dist;
            closest = (*ar, *ag, *ab);
        }
    }
    closest
}

fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS images (
            id TEXT PRIMARY KEY,
            filename TEXT NOT NULL,
            plate_x INTEGER NOT NULL,
            plate_y INTEGER NOT NULL,
            votes INTEGER DEFAULT 0,
            uploaded_at TEXT NOT NULL,
            pixel_data BLOB NOT NULL,
            width INTEGER NOT NULL,
            height INTEGER NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS votes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            image_id TEXT NOT NULL,
            voted_at TEXT NOT NULL,
            FOREIGN KEY (image_id) REFERENCES images(id)
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS coordinate_votes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            x INTEGER NOT NULL,
            y INTEGER NOT NULL,
            voted_at TEXT NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS wall_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            week INTEGER NOT NULL,
            current_target_x INTEGER,
            current_target_y INTEGER
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS wall_tiles (
            tile_x INTEGER NOT NULL,
            tile_y INTEGER NOT NULL,
            pixel_data BLOB NOT NULL,
            PRIMARY KEY (tile_x, tile_y)
        )",
        [],
    )?;

    let count: i32 = conn.query_row(
        "SELECT COUNT(*) FROM wall_state",
        [],
        |row| row.get(0),
    )?;

    let tile_count: i32 = conn.query_row(
        "SELECT COUNT(*) FROM wall_tiles",
        [],
        |row| row.get(0),
    ).unwrap_or(0);
    
    let expected_tile_count = (WALL_WIDTH_PLATES * WALL_HEIGHT_PLATES) as i32;
    
    if count == 0 || tile_count != expected_tile_count {
        eprintln!("DEBUG: Reinitializing wall tiles. Old count: {}, expected: {}", tile_count, expected_tile_count);
        
        conn.execute("DELETE FROM wall_tiles", [])?;
        
        if count == 0 {
            conn.execute("INSERT INTO wall_state (id, week, current_target_x, current_target_y) VALUES (1, 0, NULL, NULL)", [])?;
        }
        
        let white_tile = vec![255u8; PLATE_SIZE as usize * PLATE_SIZE as usize * 3];
        
        for ty in 0..WALL_HEIGHT_PLATES {
            for tx in 0..WALL_WIDTH_PLATES {
                conn.execute(
                    "INSERT INTO wall_tiles (tile_x, tile_y, pixel_data) VALUES (?1, ?2, ?3)",
                    params![tx as i32, ty as i32, &white_tile],
                )?;
            }
        }
        
        eprintln!("DEBUG: Wall tiles initialized!");
    }

    Ok(())
}

fn get_wall_state(conn: &Connection) -> WallState {
    let week: u32 = conn
        .query_row(
            "SELECT week FROM wall_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let current_target = match conn.query_row(
        "SELECT current_target_x, current_target_y FROM wall_state WHERE id = 1",
        [],
        |row| {
            let x: Option<i32> = row.get(0)?;
            let y: Option<i32> = row.get(1)?;
            Ok((x, y))
        },
    ) {
        Ok((Some(x), Some(y))) => Some(TargetCoordinate { x: x as u32, y: y as u32 }),
        _ => None,
    };

    let mut stmt = conn
        .prepare("SELECT tile_x, tile_y, pixel_data FROM wall_tiles ORDER BY tile_y, tile_x")
        .unwrap();
    
    let mut tiles: HashMap<(i32, i32), Vec<u8>> = HashMap::new();
    let rows = stmt.query_map([], |row| {
        let tx: i32 = row.get(0)?;
        let ty: i32 = row.get(1)?;
        let data: Vec<u8> = row.get(2)?;
        Ok((tx, ty, data))
    }).unwrap();
    
    for row in rows {
        if let Ok((tx, ty, data)) = row {
            tiles.insert((tx, ty), data);
        }
    }
    
    let mut pixels = vec![255u8; (TOTAL_WIDTH * TOTAL_HEIGHT * 3) as usize];
    
    for ty in 0..WALL_HEIGHT_PLATES as i32 {
        for tx in 0..WALL_WIDTH_PLATES as i32 {
            if let Some(tile_data) = tiles.get(&(tx, ty)) {
                for py in 0..PLATE_SIZE {
                    for px in 0..PLATE_SIZE {
                        let tile_pixel_idx = ((py * PLATE_SIZE + px) * 3) as usize;
                        let wall_pixel_idx = (((ty * PLATE_SIZE as i32 + py as i32) * TOTAL_WIDTH as i32 
                            + (tx * PLATE_SIZE as i32 + px as i32)) * 3) as usize;
                        
                        if tile_pixel_idx + 2 < tile_data.len() && wall_pixel_idx + 2 < pixels.len() {
                            pixels[wall_pixel_idx] = tile_data[tile_pixel_idx];
                            pixels[wall_pixel_idx + 1] = tile_data[tile_pixel_idx + 1];
                            pixels[wall_pixel_idx + 2] = tile_data[tile_pixel_idx + 2];
                        }
                    }
                }
            }
        }
    }

    WallState {
        week,
        current_target,
        pixels,
    }
}

fn render_wall_preview(state: &WallState) -> Vec<u8> {
    let mut img = image::RgbImage::new(TOTAL_WIDTH, TOTAL_HEIGHT);
    
    for (i, pixel) in state.pixels.chunks(3).enumerate() {
        let x = (i as u32) % TOTAL_WIDTH;
        let y = (i as u32) / TOTAL_WIDTH;
        if pixel.len() == 3 {
            img.put_pixel(x, y, image::Rgb([pixel[0], pixel[1], pixel[2]]));
        }
    }

    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

async fn get_wall(data: web::Data<AppState>) -> impl Responder {
    let conn = data.db.lock().unwrap();
    let state = get_wall_state(&conn);
    
    let png_data = render_wall_preview(&state);
    let base64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_data);
    
    let json_state = serde_json::json!({
        "week": state.week,
        "current_target": state.current_target,
        "image": format!("data:image/png;base64,{}", base64_data),
        "dimensions": {
            "width_plates": WALL_WIDTH_PLATES,
            "height_plates": WALL_HEIGHT_PLATES,
            "plate_size": PLATE_SIZE,
            "total_width": TOTAL_WIDTH,
            "total_height": TOTAL_HEIGHT
        }
    });
    
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json_state)
}

async fn list_images(data: web::Data<AppState>) -> impl Responder {
    let conn = data.db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT id, filename, plate_x, plate_y, votes, uploaded_at, width, height FROM images ORDER BY votes DESC")
        .unwrap();
    
    let mut images: Vec<UploadedImage> = Vec::new();
    let rows = stmt.query_map([], |row| {
        Ok(UploadedImage {
            id: row.get(0)?,
            filename: row.get(1)?,
            plate_x: row.get::<_, i32>(2)? as u32,
            plate_y: row.get::<_, i32>(3)? as u32,
            votes: row.get::<_, i32>(4)? as u32,
            uploaded_at: row.get(5)?,
            width: row.get::<_, i32>(6)? as u32,
            height: row.get::<_, i32>(7)? as u32,
            pixel_data: Vec::new(),
        })
    }).unwrap();

    for row in rows {
        if let Ok(mut img) = row {
            if let Ok(pixels) = conn.query_row::<Vec<u8>, _, _>(
                "SELECT pixel_data FROM images WHERE id = ?1",
                [&img.id],
                |row| row.get(0),
            ) {
                img.pixel_data = pixels;
            }
            images.push(img);
        }
    }

    let response: Vec<serde_json::Value> = images
        .iter()
        .map(|img| {
            let preview = create_image_preview(&img.pixel_data, img.width, img.height);
            let base64_preview = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &preview);
            
            serde_json::json!({
                "id": img.id,
                "filename": img.filename,
                "plate_x": img.plate_x,
                "plate_y": img.plate_y,
                "votes": img.votes,
                "uploaded_at": img.uploaded_at,
                "width": img.width,
                "height": img.height,
                "preview": format!("data:image/png;base64,{}", base64_preview)
            })
        })
        .collect();

    HttpResponse::Ok()
        .content_type("application/json")
        .json(response)
}

fn create_image_preview(pixel_data: &[u8], width: u32, height: u32) -> Vec<u8> {
    if pixel_data.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }

    let mut img = image::RgbImage::new(width, height);
    
    for (i, chunk) in pixel_data.chunks(3).enumerate() {
        let x = (i as u32) % width;
        let y = (i as u32) / width;
        if chunk.len() == 3 {
            img.put_pixel(x, y, image::Rgb([chunk[0], chunk[1], chunk[2]]));
        }
    }

    let mut buf = Vec::new();
    let _ = img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png);
    buf
}

fn parse_bmp(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if data.len() < 54 {
        return None;
    }
    
    let width = u32::from_le_bytes([data[18], data[19], data[20], data[21]]);
    let height = u32::from_le_bytes([data[22], data[23], data[24], data[25]]);
    let bpp = u16::from_le_bytes([data[28], data[29]]);
    
    if bpp != 24 {
        return None;
    }
    
    let data_offset = u32::from_le_bytes([data[10], data[11], data[12], data[13]]);
    let image_data = &data[data_offset as usize..];
    
    let mut pixels = Vec::new();
    let abs_height = height;
    
    for y in 0..abs_height {
        let row_offset = (y * ((width * 3 + 3) & !3u32)) as usize;
        for x in 0..width {
            let pixel_offset = row_offset + (x * 3) as usize;
            if pixel_offset + 3 <= image_data.len() {
                let b = image_data[pixel_offset];
                let g = image_data[pixel_offset + 1];
                let r = image_data[pixel_offset + 2];
                pixels.push(r);
                pixels.push(g);
                pixels.push(b);
            }
        }
    }
    
    Some((width, abs_height, pixels))
}

async fn upload_image(mut payload: Multipart, data: web::Data<AppState>) -> impl Responder {
    let mut filename = String::new();
    let mut pixel_data: Vec<u8> = Vec::new();
    let mut width: u32 = 0;
    let mut height: u32 = 0;

    while let Some(item) = payload.next().await {
        let mut field = match item {
            Ok(f) => f,
            Err(e) => {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": format!("Error reading upload: {}", e),
                    "hint": "Make sure the file is valid and try again"
                }));
            }
        };
        let content_disposition = field.content_disposition();
        
        if let Some(cd) = content_disposition.as_ref() {
            if let Some(name) = cd.get_filename() {
                filename = name.to_string();
            }
        }

        if filename.to_lowercase().ends_with(".bmp") || filename.to_lowercase().ends_with(".png") {
            let mut data_bytes = Vec::new();
            while let Some(chunk_result) = field.next().await {
                match chunk_result {
                    Ok(chunk) => data_bytes.extend_from_slice(&chunk),
                    Err(e) => {
                        return HttpResponse::BadRequest().json(serde_json::json!({
                            "error": format!("Error reading file data: {}", e)
                        }));
                    }
                }
            }

            if filename.to_lowercase().ends_with(".bmp") {
                if let Some((w, h, pixels)) = parse_bmp(&data_bytes) {
                    width = w;
                    height = h;
                    for chunk in pixels.chunks(3) {
                        if chunk.len() == 3 {
                            let (r, g, b) = find_closest_approved_color(chunk[0], chunk[1], chunk[2]);
                            pixel_data.push(r);
                            pixel_data.push(g);
                            pixel_data.push(b);
                        }
                    }
                }
            } else if let Ok(img) = image::load_from_memory(&data_bytes) {
                let rgb = img.to_rgb8();
                width = rgb.width();
                height = rgb.height();

                for y in 0..height {
                    for x in 0..width {
                        let pixel = rgb.get_pixel(x, y);
                        let (r, g, b) = find_closest_approved_color(pixel[0], pixel[1], pixel[2]);
                        pixel_data.push(r);
                        pixel_data.push(g);
                        pixel_data.push(b);
                    }
                }
            }
        }
    }

    if pixel_data.is_empty() {
        let debug_info = if filename.is_empty() {
            "No filename found in upload".to_string()
        } else if !filename.to_lowercase().ends_with(".bmp") && !filename.to_lowercase().ends_with(".png") {
            format!("File '{}' is not .bmp or .png", filename)
        } else {
            format!("Failed to parse image '{}'. Is it a valid BMP (24-bit) or PNG?", filename)
        };
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": debug_info,
            "filename": filename,
            "hint": "Upload a 30x30 pixel BMP or PNG image with the file extension .bmp or .png"
        }));
    }

    if width != PLATE_SIZE || height != PLATE_SIZE {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("Image must be exactly {}x{} pixels (one plate)", PLATE_SIZE, PLATE_SIZE),
            "received": {
                "width": width,
                "height": height
            },
            "required": {
                "width": PLATE_SIZE,
                "height": PLATE_SIZE
            }
        }));
    }

    let id = Uuid::new_v4().to_string();
    let uploaded_at = chrono::Utc::now().to_rfc3339();
    
    let conn = data.db.lock().unwrap();
    
    let (plate_x, plate_y) = match conn.query_row(
        "SELECT current_target_x, current_target_y FROM wall_state WHERE id = 1",
        [],
        |row| {
            let x: Option<i32> = row.get(0)?;
            let y: Option<i32> = row.get(1)?;
            Ok((x, y))
        },
    ) {
        Ok((Some(tx), Some(ty))) => (tx as u32, ty as u32),
        _ => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "No target coordinate set for this week. Vote for a coordinate first.",
                "hint": "Click on the grid above to vote for which plate should be updated next week."
            }));
        }
    };

    conn.execute(
        "INSERT INTO images (id, filename, plate_x, plate_y, votes, uploaded_at, pixel_data, width, height) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8)",
        params![id, filename, plate_x as i32, plate_y as i32, uploaded_at, pixel_data.clone(), width as i32, height as i32],
    ).unwrap();

    let preview = create_image_preview(&pixel_data, width, height);
    let base64_preview = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &preview);

    HttpResponse::Ok().json(serde_json::json!({
        "id": id,
        "filename": filename,
        "plate_x": plate_x,
        "plate_y": plate_y,
        "uploaded_at": uploaded_at,
        "preview": format!("data:image/png;base64,{}", base64_preview),
        "message": "Image uploaded successfully"
    }))
}

async fn vote_image(req: web::Json<VoteRequest>, data: web::Data<AppState>) -> impl Responder {
    let conn = data.db.lock().unwrap();
    
    let rows = conn.execute(
        "UPDATE images SET votes = votes + 1 WHERE id = ?1",
        [&req.image_id],
    ).unwrap();

    if rows == 0 {
        return HttpResponse::NotFound().json(serde_json::json!({"error": "Image not found"}));
    }

    let voted_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO votes (image_id, voted_at) VALUES (?1, ?2)",
        params![req.image_id, voted_at],
    ).unwrap();

    let votes: i32 = conn.query_row(
        "SELECT votes FROM images WHERE id = ?1",
        [&req.image_id],
        |row| row.get(0),
    ).unwrap_or(0);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "image_id": req.image_id,
        "votes": votes
    }))
}

async fn vote_coordinate(req: web::Json<CoordinateVoteRequest>, data: web::Data<AppState>) -> impl Responder {
    if req.x >= WALL_WIDTH_PLATES || req.y >= WALL_HEIGHT_PLATES {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("Coordinates must be within wall bounds (0-{}, 0-{}) in plates", WALL_WIDTH_PLATES - 1, WALL_HEIGHT_PLATES - 1)
        }));
    }

    let conn = data.db.lock().unwrap();
    let voted_at = chrono::Utc::now().to_rfc3339();
    
    conn.execute(
        "INSERT INTO coordinate_votes (x, y, voted_at) VALUES (?1, ?2, ?3)",
        params![req.x as i32, req.y as i32, voted_at],
    ).unwrap();

    let vote_count: i32 = conn.query_row(
        "SELECT COUNT(*) FROM coordinate_votes WHERE x = ?1 AND y = ?2",
        params![req.x as i32, req.y as i32],
        |row| row.get(0),
    ).unwrap_or(0);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "x": req.x,
        "y": req.y,
        "vote_count": vote_count
    }))
}

async fn get_coordinate_votes(data: web::Data<AppState>) -> impl Responder {
    let conn = data.db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT x, y, COUNT(*) as votes FROM coordinate_votes GROUP BY x, y ORDER BY votes DESC")
        .unwrap();
    
    let mut votes: Vec<Value> = Vec::new();
    let rows = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "x": row.get::<_, i32>(0)?,
            "y": row.get::<_, i32>(1)?,
            "votes": row.get::<_, i32>(2)?
        }))
    }).unwrap();

    for row in rows {
        if let Ok(v) = row {
            votes.push(v);
        }
    }

    HttpResponse::Ok()
        .content_type("application/json")
        .json(votes)
}

async fn advance_week(data: web::Data<AppState>) -> impl Responder {
    let conn = data.db.lock().unwrap();
    
    let current_week: u32 = conn.query_row(
        "SELECT week FROM wall_state WHERE id = 1",
        [],
        |row| row.get(0),
    ).unwrap_or(0);

    let winner: Option<(i32, i32)> = conn.query_row(
        "SELECT x, y FROM coordinate_votes GROUP BY x, y ORDER BY COUNT(*) DESC LIMIT 1",
        [],
        |row| Ok((row.get::<_, i32>(0)?, row.get::<_, i32>(1)?))
    ).ok();

    println!("winner {:?} {:?}", winner, winner.is_none());
    if winner.is_none() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "No coordinate has been voted on yet. Vote for a coordinate first."
        }));
    }

    let winner_image_id: Option<String> = conn.query_row(
        "SELECT id FROM images ORDER BY votes DESC LIMIT 1",
        [],
        |row| row.get(0)
    ).ok();

    let new_week = current_week + 1;

    let old_target: Option<(i32, i32)> = match conn.query_row(
        "SELECT current_target_x, current_target_y FROM wall_state WHERE id = 1",
        [],
        |row| {
            let x: Option<i32> = row.get(0)?;
            let y: Option<i32> = row.get(1)?;
            Ok((x, y))
        },
    ) {
        Ok((Some(x), Some(y))) => Some((x, y)),
        _ => None,
    };

    if let Some((old_x, old_y)) = old_target {
        if let Some(img_id) = &winner_image_id {
            if let Ok(pixel_data) = conn.query_row::<Vec<u8>, _, _>(
                "SELECT pixel_data FROM images WHERE id = ?1",
                [img_id],
                |row| row.get(0),
            ) {
                if !pixel_data.is_empty() {
                    conn.execute(
                        "UPDATE wall_tiles SET pixel_data = ?1 WHERE tile_x = ?2 AND tile_y = ?3",
                        params![&pixel_data, old_x, old_y],
                    ).unwrap();
                }
            }
        }
    }

    if let Some((wx, wy)) = winner {
        conn.execute(
            "UPDATE wall_state SET week = ?1, current_target_x = ?2, current_target_y = ?3 WHERE id = 1",
            params![new_week, wx, wy],
        ).unwrap();
    } else {
        conn.execute(
            "UPDATE wall_state SET week = ?1, current_target_x = NULL, current_target_y = NULL WHERE id = 1",
            params![new_week],
        ).unwrap();
    }

    conn.execute("DELETE FROM coordinate_votes", []).unwrap();
    conn.execute("DELETE FROM votes", []).unwrap();
    conn.execute("DELETE FROM images", []).unwrap();

    let state = get_wall_state(&conn);
    let png_data = render_wall_preview(&state);
    let base64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_data);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "week": new_week,
        "winner_coordinate": winner.map(|(x, y)| serde_json::json!({"x": x, "y": y})),
        "winner_image": winner_image_id,
        "wall_image": format!("data:image/png;base64,{}", base64_data)
    }))
}

async fn get_stats(data: web::Data<AppState>) -> impl Responder {
    let conn = data.db.lock().unwrap();
    
    let total_votes: i32 = conn.query_row(
        "SELECT COUNT(*) FROM votes",
        [],
        |row| row.get(0),
    ).unwrap_or(0);

    let total_coordinate_votes: i32 = conn.query_row(
        "SELECT COUNT(*) FROM coordinate_votes",
        [],
        |row| row.get(0),
    ).unwrap_or(0);

    let total_images: i32 = conn.query_row(
        "SELECT COUNT(*) FROM images",
        [],
        |row| row.get(0),
    ).unwrap_or(0);

    let week: u32 = conn.query_row(
        "SELECT week FROM wall_state WHERE id = 1",
        [],
        |row| row.get(0),
    ).unwrap_or(0);

    let current_target = match conn.query_row(
        "SELECT current_target_x, current_target_y FROM wall_state WHERE id = 1",
        [],
        |row| {
            let x: Option<i32> = row.get(0)?;
            let y: Option<i32> = row.get(1)?;
            Ok((x, y))
        },
    ) {
        Ok((Some(x), Some(y))) => Some(serde_json::json!({"x": x, "y": y})),
        _ => None,
    };

    HttpResponse::Ok().json(serde_json::json!({
        "week": week,
        "current_target": current_target,
        "total_image_votes": total_votes,
        "total_coordinate_votes": total_coordinate_votes,
        "total_images": total_images,
        "wall_dimensions": {
            "plates_width": WALL_WIDTH_PLATES,
            "plates_height": WALL_HEIGHT_PLATES,
            "plate_size": PLATE_SIZE,
            "total_width": TOTAL_WIDTH,
            "total_height": TOTAL_HEIGHT
        }
    }))
}

async fn index() -> impl Responder {
    let html = r#"<!DOCTYPE html>
<html lang="sv">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Pärlplattvotering</title>
    <style>
        body { font-family: Arial, sans-serif; max-width: 1200px; margin: 0 auto; padding: 20px; background: #f5f5f5; }
        h1, h2, h3 { color: #333; }
        .section { background: white; padding: 20px; margin: 20px 0; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        .wall-container { text-align: center; }
        .wall-image { max-width: 100%; border: 2px solid #333; border-radius: 4px; }
        .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 15px; }
        .image-card { border: 1px solid #ddd; padding: 10px; border-radius: 4px; background: #fafafa; }
        .image-card img { width: 100%; height: 150px; object-fit: contain; background: #eee; }
        .image-card h4 { margin: 10px 0 5px; font-size: 14px; word-break: break-all; }
        .image-card p { margin: 5px 0; color: #666; }
        .vote-btn { background: #4CAF50; color: white; border: none; padding: 10px 20px; cursor: pointer; border-radius: 4px; width: 100%; }
        .vote-btn:hover { background: #45a049; }
        .admin-btn { background: #ff9800; color: white; border: none; padding: 15px 30px; cursor: pointer; border-radius: 4px; font-size: 16px; }
        .admin-btn:hover { background: #f57c00; }
        .coord-grid { display: grid; grid-template-columns: repeat(27, 1fr); gap: 1px; max-width: 100%; }
        .coord-cell { aspect-ratio: 1; border: 1px solid #ccc; cursor: pointer; font-size: 8px; display: flex; align-items: center; justify-content: center; background: white; font-family: monospace; }
        .coord-cell:hover { background: #e0e0e0; }
        .coord-cell.target { background: #ffeb3b; border: 2px solid #f57c00; }
        .coord-cell.has-votes { background: #c8e6c9; }
        form { margin: 10px 0; }
        input[type="file"] { margin: 10px 0; }
        input[type="submit"] { background: #2196F3; color: white; border: none; padding: 10px 20px; cursor: pointer; border-radius: 4px; }
        input[type="submit"]:hover { background: #1976D2; }
        .stats { display: flex; gap: 20px; flex-wrap: wrap; }
        .stat-box { background: #e3f2fd; padding: 15px; border-radius: 4px; text-align: center; }
        .stat-box .value { font-size: 24px; font-weight: bold; color: #1976D2; }
        .stat-box .label { color: #666; }
        .approved-colors { display: flex; gap: 10px; flex-wrap: wrap; margin: 10px 0; }
        .color-swatch { width: 30px; height: 30px; border: 1px solid #333; border-radius: 4px; }
    </style>
</head>
<body>
    <h1>Pärlplattvotering</h1>
    
    <div class="section stats">
        <div class="stat-box">
            <div class="value" id="week">-</div>
            <div class="label">Vecka</div>
        </div>
        <div class="stat-box">
            <div class="value" id="totalImages">-</div>
            <div class="label">Antal bilder</div>
        </div>
        <div class="stat-box">
            <div class="value" id="totalVotes">-</div>
            <div class="label">Bildröster</div>
        </div>
        <div class="stat-box">
            <div class="value" id="coordVotes">-</div>
            <div class="label">Koordinatröster</div>
        </div>
    </div>

    <div class="section">
        <h2>Godkända färger</h2>
        <div class="approved-colors" id="colorPalette"></div>
    </div>

    <div class="section wall-container">
        <h2>Väggen hittills</h2>
        <p>Nuvarande mål: <span id="currentTarget">Ingen koordinat vald</span></p>
        <img id="wallImage" class="wall-image" src="" alt="Väggen">
    </div>

    <div class="section">
        <h2>Rösta på koordinat (plattor)</h2>
        <p>Klicka på en platta (27x12) för att rösta på vilken koordinat som ska uppdateras härnäst:</p>
        <div class="coord-grid" id="coordGrid"></div>
    </div>

    <div class="section">
        <h2>Ladda upp bild</h2>
        <form id="uploadForm" enctype="multipart/form-data">
            <input type="file" name="file" accept=".bmp,.png" required>
            <p>Bild ska vara exakt 30x30 pixlar med godkända färger.</p>
            <input type="submit" value="Ladda upp">
        </form>
        <div id="uploadResult"></div>
    </div>

    <div class="section">
        <h2>Rösta på bild</h2>
        <p>Välj vilken bild som ska läggas upp på den valda koordinaten:</p>
        <div class="grid" id="imageGrid"></div>
    </div>

    <div class="section">
        <h2>Admin</h2>
        <button class="admin-btn" onclick="advanceWeek()">Ny vecka (debug)</button>
        <div id="adminResult"></div>
    </div>

    <script>
        const colors = [
            {name: 'VIT', r: 255, g: 255, b: 255},
            {name: 'GUL', r: 255, g: 255, b: 0},
            {name: 'ORANGE', r: 255, g: 165, b: 0},
            {name: 'RÖD', r: 255, g: 0, b: 0},
            {name: 'LILA', r: 128, g: 0, b: 128},
            {name: 'ROSA', r: 255, g: 192, b: 203},
            {name: 'BLÅ', r: 0, g: 0, b: 255},
            {name: 'LJUSGRÖN', r: 144, g: 238, b: 144},
            {name: 'BRUN', r: 139, g: 69, b: 19},
            {name: 'SVART', r: 0, g: 0, b: 0}
        ];

        function renderColorPalette() {
            const container = document.getElementById('colorPalette');
            container.innerHTML = colors.map(c => 
                `<div class="color-swatch" style="background: rgb(${c.r},${c.g},${c.b})" title="${c.name}"></div>`
            ).join('');
        }

        async function loadStats() {
            const res = await fetch('/api/stats');
            const data = await res.json();
            document.getElementById('week').textContent = data.week;
            document.getElementById('totalImages').textContent = data.total_images;
            document.getElementById('totalVotes').textContent = data.total_image_votes;
            document.getElementById('coordVotes').textContent = data.total_coordinate_votes;
            
            if (data.current_target) {
                document.getElementById('currentTarget').textContent = 
                    `(${data.current_target.x}, ${data.current_target.y})`;
            }
        }

        async function loadWall() {
            const res = await fetch('/api/wall');
            const data = await res.json();
            document.getElementById('wallImage').src = data.image;
        }

        async function loadImages() {
            const res = await fetch('/api/images');
            const images = await res.json();
            const grid = document.getElementById('imageGrid');
            
            if (images.length === 0) {
                grid.innerHTML = '<p>Inga bilder uppladdade ännu.</p>';
                return;
            }
            
            grid.innerHTML = images.map(img => `
                <div class="image-card">
                    <img src="${img.preview}" alt="${img.filename}">
                    <h4>${img.filename}</h4>
                    <p>Position: (${img.plate_x}, ${img.plate_y})</p>
                    <p>Röster: <strong>${img.votes}</strong></p>
                    <button class="vote-btn" onclick="voteImage('${img.id}')">Rösta</button>
                </div>
            `).join('');
        }

        async function loadCoordinateVotes() {
            const res = await fetch('/api/coordinates/votes');
            const votes = await res.json();
            const voteMap = {};
            votes.forEach(v => { voteMap[`${v.x},${v.y}`] = v.votes; });
            
            const wallRes = await fetch('/api/wall');
            const wallData = await wallRes.json();
            const target = wallData.current_target;
            
            const grid = document.getElementById('coordGrid');
            let html = '';
            for (let y = 0; y < 12; y++) {
                for (let x = 0; x < 27; x++) {
                    const key = `${x},${y}`;
                    const isTarget = target && target.x === x && target.y === y;
                    const votes = voteMap[key] || 0;
                    html += `<div class="coord-cell ${isTarget ? 'target' : ''} ${votes > 0 && !isTarget ? 'has-votes' : ''}" 
                        onclick="voteCoordinate(${x}, ${y})" title="Plate (${x}, ${y}): ${votes} röster">${votes > 0 ? votes : ''}</div>`;
                }
            }
            grid.innerHTML = html;
        }

        async function voteImage(imageId) {
            const res = await fetch('/api/vote', {
                method: 'POST',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({image_id: imageId})
            });
            const data = await res.json();
            if (data.success) {
                loadImages();
                loadStats();
            }
        }

        async function voteCoordinate(x, y) {
            const res = await fetch('/api/coordinate/vote', {
                method: 'POST',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({x, y})
            });
            const data = await res.json();
            if (data.success) {
                loadCoordinateVotes();
                loadStats();
            }
        }

        async function advanceWeek() {
            if (!confirm('Är du säker på att du vill gå till nästa vecka?')) return;
            const res = await fetch('/api/admin/advance', {method: 'POST'});
            const data = await res.json();
            document.getElementById('adminResult').innerHTML = `
                <p>Vecka ${data.week} har startat!</p>
                <p>Vinnande koordinat: ${data.winner_coordinate ? `(${data.winner_coordinate.x}, ${data.winner_coordinate.y})` : 'Ingen'}</p>
            `;
            loadWall();
            loadImages();
            loadCoordinateVotes();
            loadStats();
        }

        document.getElementById('uploadForm').onsubmit = async (e) => {
            e.preventDefault();
            const formData = new FormData(e.target);
            const res = await fetch('/api/upload', {method: 'POST', body: formData});
            const data = await res.json();
            const result = document.getElementById('uploadResult');
            if (data.error) {
                result.innerHTML = `<p style="color: red;">Fel: ${data.error}</p>`;
            } else {
                result.innerHTML = `<p style="color: green;">${data.message} (${data.filename})</p>`;
                loadImages();
                loadStats();
            }
            e.target.reset();
        };

        renderColorPalette();
        loadStats();
        loadWall();
        loadImages();
        loadCoordinateVotes();
    </script>
</body>
</html>"#;
    
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let conn = Connection::open("wall.db").expect("Failed to open database");
    init_db(&conn).expect("Failed to initialize database");

    let app_state = web::Data::new(AppState {
        db: Mutex::new(conn),
    });

    println!("Starting server at http://127.0.0.1:8080");
    println!("Wall dimensions: {}x{} plates ({}x{} pixels)", 
             WALL_WIDTH_PLATES, WALL_HEIGHT_PLATES, TOTAL_WIDTH, TOTAL_HEIGHT);

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .route("/", web::get().to(index))
            .route("/api/wall", web::get().to(get_wall))
            .route("/api/images", web::get().to(list_images))
            .route("/api/upload", web::post().to(upload_image))
            .route("/api/vote", web::post().to(vote_image))
            .route("/api/coordinate/vote", web::post().to(vote_coordinate))
            .route("/api/coordinates/votes", web::get().to(get_coordinate_votes))
            .route("/api/stats", web::get().to(get_stats))
            .route("/api/admin/advance", web::post().to(advance_week))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
