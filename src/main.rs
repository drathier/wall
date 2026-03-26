use actix_multipart::Multipart;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::Path;
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

const CACHE_DIR: &str = "./cache";
const EVENTS_FILE: &str = "wall.jsonl";

struct AppState {
    events_path: Mutex<String>,
}

fn ensure_cache_dir() {
    if !Path::new(CACHE_DIR).exists() {
        fs::create_dir_all(CACHE_DIR).expect("Failed to create cache directory");
    }
}

fn clear_cache() {
    ensure_cache_dir();
    if let Ok(entries) = fs::read_dir(CACHE_DIR) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn get_cached_wall_path() -> String {
    format!("{}/wall.png", CACHE_DIR)
}

fn save_cached_wall(png_data: &[u8]) {
    let path = get_cached_wall_path();
    let _ = fs::write(&path, png_data);
}

fn get_cached_wall() -> Option<Vec<u8>> {
    let path = get_cached_wall_path();
    fs::read(&path).ok()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonLineEvent {
    event_type: String,
    event_id: String,
    payload: Value,
    created_at: String,
}

fn read_all_events(events_path: &str) -> Vec<JsonLineEvent> {
    let mut events = Vec::new();
    if let Ok(file) = fs::File::open(events_path) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            if let Ok(event) = serde_json::from_str::<JsonLineEvent>(&line) {
                events.push(event);
            }
        }
    }
    events
}

fn append_json_event(events_path: &str, event_type: &str, payload: &Value) -> String {
    let event_id = Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    
    let event = JsonLineEvent {
        event_type: event_type.to_string(),
        event_id: event_id.clone(),
        payload: payload.clone(),
        created_at,
    };
    
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(events_path)
        .expect("Failed to open events file");
    
    let json = serde_json::to_string(&event).unwrap();
    writeln!(file, "{}", json).expect("Failed to write event");
    
    event_id
}

fn get_events_by_type<T: for<'de> Deserialize<'de>>(events_path: &str, event_type: &str) -> Vec<(String, T)> {
    println!("[DEBUG READ] get_events_by_type: type={}", event_type);
    
    let events = read_all_events(events_path);
    
    let results: Vec<(String, T)> = events
        .into_iter()
        .filter(|e| e.event_type == event_type)
        .map(|e| {
            let payload: T = serde_json::from_value(e.payload).unwrap();
            (e.event_id, payload)
        })
        .collect();
    
    println!("[DEBUG READ] get_events_by_type: type={}, count={}", event_type, results.len());
    results
}

fn get_all_events_json(events_path: &str) -> Vec<(String, String, Value, String)> {
    let events = read_all_events(events_path);
    events
        .into_iter()
        .map(|e| (e.event_type, e.event_id, e.payload, e.created_at))
        .collect()
}

fn get_latest_week_advanced(events_path: &str) -> Option<WeekAdvancedEvent> {
    let events = read_all_events(events_path);
    events
        .into_iter()
        .filter(|e| e.event_type == "week_advanced")
        .last()
        .map(|e| serde_json::from_value(e.payload).unwrap())
}

fn get_current_week(events_path: &str) -> u32 {
    get_latest_week_advanced(events_path)
        .map(|e| e.to_week)
        .unwrap_or(0)
}

fn get_current_target(events_path: &str) -> Option<(i32, i32)> {
    get_latest_week_advanced(events_path)
        .map(|e| (e.next_target_x, e.next_target_y))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUploadedEvent {
    pub event_id: String,
    pub week: u32,
    pub target_x: i32,
    pub target_y: i32,
    pub filename: String,
    pub pixel_data: Vec<u8>,
    pub uploaded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageVotedEvent {
    pub event_id: String,
    pub image_event_id: String,
    pub voted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinateVotedEvent {
    pub event_id: String,
    pub week: u32,
    pub x: i32,
    pub y: i32,
    pub voted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeekAdvancedEvent {
    pub event_id: String,
    pub from_week: u32,
    pub to_week: u32,
    pub applied_x: i32,
    pub applied_y: i32,
    pub winning_image_event_id: Option<String>,
    pub next_target_x: i32,
    pub next_target_y: i32,
    pub applied_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiClickedEvent {
    pub event_id: String,
    pub click_type: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub target: Option<String>,
    pub clicked_at: String,
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

fn create_tile_from_color(color: &[u8]) -> Vec<u8> {
    let (r, g, b) = if color.len() >= 3 {
        (color[0], color[1], color[2])
    } else {
        (255, 255, 255)
    };
    let mut tile = Vec::with_capacity((PLATE_SIZE * PLATE_SIZE * 3) as usize);
    for _ in 0..(PLATE_SIZE * PLATE_SIZE) {
        tile.push(r);
        tile.push(g);
        tile.push(b);
    }
    tile
}



fn get_default_tile_pattern() -> HashMap<(i32, i32), Vec<u8>> {
    let colors: Vec<(u8, u8, u8)> = APPROVED_COLORS
        .iter()
        .map(|(_, r, g, b)| (*r, *g, *b))
        .collect();
    
    let num_colors = colors.len() as i32;
    
    let mut tiles: HashMap<(i32, i32), Vec<u8>> = HashMap::new();
    
    for ty in 0..WALL_HEIGHT_PLATES as i32 {
        for tx in 0..WALL_WIDTH_PLATES as i32 {
            let color_idx = (((tx as i32) + (ty as i32) * 3).abs() % num_colors) as usize;
            let color = colors[color_idx];
            let tile = create_tile_from_color(&[color.0, color.1, color.2]);
            tiles.insert((tx, ty), tile);
        }
    }
    
    tiles
}

fn get_wall_tiles(events_path: &str) -> HashMap<(i32, i32), Vec<u8>> {
    let mut tiles = get_default_tile_pattern();
    
    let week_events: Vec<WeekAdvancedEvent> = get_events_by_type(events_path, "week_advanced")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    
    println!("[DEBUG WALL] get_wall_tiles: week_advanced events count: {}", week_events.len());
    
    let image_events: HashMap<String, ImageUploadedEvent> = get_events_by_type::<ImageUploadedEvent>(events_path, "image_uploaded")
        .into_iter()
        .map(|(_, e)| (e.event_id.clone(), e))
        .collect();
    
    println!("[DEBUG WALL] get_wall_tiles: image_uploaded events count: {}", image_events.len());
    
    for event in &week_events {
        println!("[DEBUG WALL] Processing week_advanced: from={}, to={}, applied=({},{}), winning_image={:?}", 
            event.from_week, event.to_week, event.applied_x, event.applied_y, event.winning_image_event_id);
        if event.applied_x >= 0 && event.applied_y >= 0 {
            if let Some(img_event_id) = &event.winning_image_event_id {
                if let Some(img_event) = image_events.get(img_event_id) {
                    println!("[DEBUG WALL] Applying image {} to tile ({}, {})", img_event_id, event.applied_x, event.applied_y);
                    tiles.insert((event.applied_x, event.applied_y), img_event.pixel_data.clone());
                } else {
                    println!("[DEBUG WALL] WARNING: image_event_id {} not found in image_events!", img_event_id);
                }
            }
        }
    }
    
    tiles
}

fn get_current_week_images(events_path: &str, week: u32) -> Vec<ImageUploadedEvent> {
    get_events_by_type::<ImageUploadedEvent>(events_path, "image_uploaded")
        .into_iter()
        .map(|(_, e)| e)
        .filter(|e| e.week == week)
        .collect()
}

fn get_wall_tiles_for_week(events_path: &str, target_week: u32) -> HashMap<(i32, i32), Vec<u8>> {
    let mut tiles = get_default_tile_pattern();
    
    let week_events: Vec<WeekAdvancedEvent> = get_events_by_type::<WeekAdvancedEvent>(events_path, "week_advanced")
        .into_iter()
        .map(|(_, e)| e)
        .filter(|e| e.to_week <= target_week)
        .collect();
    
    let image_events: HashMap<String, ImageUploadedEvent> = get_events_by_type::<ImageUploadedEvent>(events_path, "image_uploaded")
        .into_iter()
        .map(|(_, e)| (e.event_id.clone(), e))
        .collect();
    
    for event in &week_events {
        if event.applied_x >= 0 && event.applied_y >= 0 {
            if let Some(img_event_id) = &event.winning_image_event_id {
                if let Some(img_event) = image_events.get(img_event_id) {
                    tiles.insert((event.applied_x, event.applied_y), img_event.pixel_data.clone());
                }
            }
        }
    }
    
    tiles
}

fn render_wall_preview_for_week(events_path: &str, target_week: u32) -> Vec<u8> {
    let tiles = get_wall_tiles_for_week(events_path, target_week);
    
    let mut pixels = vec![255u8; (TOTAL_WIDTH * TOTAL_HEIGHT * 3) as usize];
    
    for ty in 0..WALL_HEIGHT_PLATES as i32 {
        for tx in 0..WALL_WIDTH_PLATES as i32 {
            if let Some(tile_data) = tiles.get(&(tx, ty)) {
                let ty = ty as usize;
                let tx = tx as usize;
                let row_size = (PLATE_SIZE * 3) as usize;
                for py in 0..PLATE_SIZE as usize {
                    let src_offset = py * row_size;
                    let dest_offset = ((ty * PLATE_SIZE as usize + py) * TOTAL_WIDTH as usize + (tx * PLATE_SIZE as usize)) * 3;
                    let copy_len = row_size.min(pixels.len().saturating_sub(dest_offset));
                    if copy_len > 0 && src_offset + copy_len <= tile_data.len() {
                        pixels[dest_offset..dest_offset + copy_len].copy_from_slice(&tile_data[src_offset..src_offset + copy_len]);
                    }
                }
            }
        }
    }
    
    let img = image::RgbImage::from_raw(TOTAL_WIDTH, TOTAL_HEIGHT, pixels).expect("Failed to create image");

    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

fn get_coordinate_votes_for_week(events_path: &str, week: u32) -> Vec<CoordinateVotedEvent> {
    let events: Vec<(String, CoordinateVotedEvent)> = get_events_by_type(events_path, "coordinate_voted");
    events
        .into_iter()
        .map(|(_, e)| e)
        .filter(|e| e.week == week)
        .collect()
}

fn render_wall_preview(events_path: &str) -> Vec<u8> {
    let tiles = get_wall_tiles(events_path);
    
    let mut pixels = vec![255u8; (TOTAL_WIDTH * TOTAL_HEIGHT * 3) as usize];
    
    for ty in 0..WALL_HEIGHT_PLATES as i32 {
        for tx in 0..WALL_WIDTH_PLATES as i32 {
            if let Some(tile_data) = tiles.get(&(tx, ty)) {
                let ty = ty as usize;
                let tx = tx as usize;
                let row_size = (PLATE_SIZE * 3) as usize;
                for py in 0..PLATE_SIZE as usize {
                    let src_offset = py * row_size;
                    let dest_offset = ((ty * PLATE_SIZE as usize + py) * TOTAL_WIDTH as usize + (tx * PLATE_SIZE as usize)) * 3;
                    let copy_len = row_size.min(pixels.len().saturating_sub(dest_offset));
                    if copy_len > 0 && src_offset + copy_len <= tile_data.len() {
                        pixels[dest_offset..dest_offset + copy_len].copy_from_slice(&tile_data[src_offset..src_offset + copy_len]);
                    }
                }
            }
        }
    }
    
    let img = image::RgbImage::from_raw(TOTAL_WIDTH, TOTAL_HEIGHT, pixels).expect("Failed to create image");

    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

async fn get_wall(data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    
    let png_data = match get_cached_wall() {
        Some(data) => data,
        None => {
            let data = render_wall_preview(&events_path);
            save_cached_wall(&data);
            data
        }
    };
    let base64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_data);
    
    let current_target = get_current_target(&events_path);
    
    let json_state = serde_json::json!({
        "week": get_current_week(&events_path),
        "current_target": current_target.map(|(x, y)| serde_json::json!({"x": x, "y": y})),
        "image": format!("data:image/png;base64,{}", base64_data),
        "dimensions": {
            "width_plates": WALL_WIDTH_PLATES,
            "height_plates": WALL_HEIGHT_PLATES,
            "plate_size": PLATE_SIZE,
            "total_width": TOTAL_WIDTH,
            "total_height": TOTAL_HEIGHT
        },
        "cached": get_cached_wall().is_some()
    });
    
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json_state)
}

async fn get_wall_for_week(data: web::Data<AppState>, week: web::Path<u32>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    let target_week = *week;
    
    let png_data = render_wall_preview_for_week(&events_path, target_week);
    let base64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_data);
    
    let json_state = serde_json::json!({
        "week": target_week,
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

async fn get_images_for_week(data: web::Data<AppState>, week: web::Path<u32>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    let target_week = *week;
    
    let image_events = get_current_week_images(&events_path, target_week);
    
    let image_vote_events: Vec<ImageVotedEvent> = get_events_by_type(&events_path, "image_voted")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    
    let mut vote_counts: HashMap<String, i32> = HashMap::new();
    for vote in &image_vote_events {
        *vote_counts.entry(vote.image_event_id.clone()).or_insert(0) += 1;
    }
    
    let images: Vec<Value> = image_events
        .into_iter()
        .map(|img| {
            let votes = vote_counts.get(&img.event_id).unwrap_or(&0);
            let preview = create_image_preview(&img.pixel_data, PLATE_SIZE, PLATE_SIZE);
            let base64_preview = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &preview);
            
            serde_json::json!({
                "id": img.event_id,
                "filename": img.filename,
                "plate_x": img.target_x,
                "plate_y": img.target_y,
                "votes": votes,
                "uploaded_at": img.uploaded_at,
                "preview": format!("data:image/png;base64,{}", base64_preview)
            })
        })
        .collect();

    HttpResponse::Ok()
        .content_type("application/json")
        .json(images)
}

async fn list_images(data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    let current_week = get_current_week(&events_path);
    
    let image_events = get_current_week_images(&events_path, current_week);
    
    let image_vote_events: Vec<ImageVotedEvent> = get_events_by_type(&events_path, "image_voted")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    
    let mut vote_counts: HashMap<String, i32> = HashMap::new();
    for vote in &image_vote_events {
        *vote_counts.entry(vote.image_event_id.clone()).or_insert(0) += 1;
    }
    
    let images: Vec<Value> = image_events
        .into_iter()
        .map(|img| {
            let votes = vote_counts.get(&img.event_id).unwrap_or(&0);
            let preview = create_image_preview(&img.pixel_data, PLATE_SIZE, PLATE_SIZE);
            let base64_preview = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &preview);
            
            serde_json::json!({
                "id": img.event_id,
                "filename": img.filename,
                "plate_x": img.target_x,
                "plate_y": img.target_y,
                "votes": votes,
                "uploaded_at": img.uploaded_at,
                "preview": format!("data:image/png;base64,{}", base64_preview)
            })
        })
        .collect();

    HttpResponse::Ok()
        .content_type("application/json")
        .json(images)
}

fn create_image_preview(pixel_data: &[u8], width: u32, height: u32) -> Vec<u8> {
    if pixel_data.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }

    let img = image::RgbImage::from_raw(width, height, pixel_data.to_vec()).expect("Failed to create image");

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

    while let Some(item) = payload.next().await {
        let mut field = match item {
            Ok(f) => f,
            Err(e) => {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": format!("Error reading upload: {}", e)
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
                if let Some((w, _, pixels)) = parse_bmp(&data_bytes) {
                    width = w;
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
                let height = rgb.height();

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

    if width != PLATE_SIZE {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("Image must be exactly {}x{} pixels", PLATE_SIZE, PLATE_SIZE),
            "received": { "width": width, "height": PLATE_SIZE },
            "required": { "width": PLATE_SIZE, "height": PLATE_SIZE }
        }));
    }

    let events_path = data.events_path.lock().unwrap();
    let current_week = get_current_week(&events_path);
    let current_target = get_current_target(&events_path);
    
    let (target_x, target_y) = match current_target {
        Some((x, y)) => (x, y),
        None => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "No target coordinate set. Vote for a coordinate first.",
                "hint": "Click on the grid above to vote for which plate should be updated next week."
            }));
        }
    };

    let event = ImageUploadedEvent {
        event_id: Uuid::new_v4().to_string(),
        week: current_week,
        target_x,
        target_y,
        filename: filename.clone(),
        pixel_data,
        uploaded_at: chrono::Utc::now().to_rfc3339(),
    };

    append_json_event(&events_path, "image_uploaded", &serde_json::to_value(&event).unwrap());

    let preview = create_image_preview(&event.pixel_data, PLATE_SIZE, PLATE_SIZE);
    let base64_preview = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &preview);

    HttpResponse::Ok().json(serde_json::json!({
        "id": event.event_id,
        "filename": filename,
        "plate_x": target_x,
        "plate_y": target_y,
        "uploaded_at": event.uploaded_at,
        "preview": format!("data:image/png;base64,{}", base64_preview),
        "message": "Image uploaded successfully"
    }))
}

async fn vote_image(req: web::Json<VoteRequest>, data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    
    let image_events: Vec<ImageUploadedEvent> = get_events_by_type(&events_path, "image_uploaded")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    
    let image_exists = image_events.iter().any(|e| e.event_id == req.image_id);
    
    if !image_exists {
        return HttpResponse::NotFound().json(serde_json::json!({"error": "Image not found"}));
    }

    let event = ImageVotedEvent {
        event_id: Uuid::new_v4().to_string(),
        image_event_id: req.image_id.clone(),
        voted_at: chrono::Utc::now().to_rfc3339(),
    };

    append_json_event(&events_path, "image_voted", &serde_json::to_value(&event).unwrap());

    let image_vote_events: Vec<ImageVotedEvent> = get_events_by_type(&events_path, "image_voted")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    
    let votes = image_vote_events.iter().filter(|e| e.image_event_id == req.image_id).count() as i32;

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "image_id": req.image_id,
        "votes": votes
    }))
}

#[derive(Deserialize)]
pub struct VoteRequest {
    pub image_id: String,
}

async fn vote_coordinate(req: web::Json<CoordinateVoteRequest>, data: web::Data<AppState>) -> impl Responder {
    if req.x >= WALL_WIDTH_PLATES || req.y >= WALL_HEIGHT_PLATES {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("Coordinates must be within wall bounds (0-{}, 0-{})", WALL_WIDTH_PLATES - 1, WALL_HEIGHT_PLATES - 1)
        }));
    }

    let events_path = data.events_path.lock().unwrap();
    let current_week = get_current_week(&events_path);
    let next_week = current_week + 1;
    
    let event = CoordinateVotedEvent {
        event_id: Uuid::new_v4().to_string(),
        week: next_week,
        x: req.x as i32,
        y: req.y as i32,
        voted_at: chrono::Utc::now().to_rfc3339(),
    };

    append_json_event(&events_path, "coordinate_voted", &serde_json::to_value(&event).unwrap());

    let coord_events: Vec<(String, CoordinateVotedEvent)> = get_events_by_type(&events_path, "coordinate_voted");
    let coordinate_vote_events: Vec<CoordinateVotedEvent> = coord_events
        .into_iter()
        .map(|(_, e)| e)
        .filter(|e| e.week == next_week)
        .collect();
    
    let vote_count: i32 = coordinate_vote_events.iter()
        .filter(|e| e.x == req.x as i32 && e.y == req.y as i32)
        .count() as i32;

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "x": req.x,
        "y": req.y,
        "vote_count": vote_count
    }))
}

#[derive(Deserialize)]
pub struct CoordinateVoteRequest {
    pub x: u32,
    pub y: u32,
}

async fn get_coordinate_votes(data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    let current_week = get_current_week(&events_path);
    let next_week = current_week + 1;
    
    let coordinate_vote_events = get_coordinate_votes_for_week(&events_path, next_week);
    
    let mut vote_counts: HashMap<String, i32> = HashMap::new();
    for event in &coordinate_vote_events {
        let key = format!("{},{}", event.x, event.y);
        *vote_counts.entry(key).or_insert(0) += 1;
    }
    
    let mut votes: Vec<HashMap<String, Value>> = Vec::new();
    for (key, count) in vote_counts {
        let parts: Vec<&str> = key.split(',').collect();
        let x: i32 = parts[0].parse().unwrap();
        let y: i32 = parts[1].parse().unwrap();
        let mut vote = HashMap::new();
        vote.insert("x".to_string(), serde_json::json!(x));
        vote.insert("y".to_string(), serde_json::json!(y));
        vote.insert("votes".to_string(), serde_json::json!(count));
        votes.push(vote);
    }
    
    votes.sort_by(|a, b| {
        let a_votes = a.get("votes").unwrap().as_i64().unwrap();
        let b_votes = b.get("votes").unwrap().as_i64().unwrap();
        b_votes.cmp(&a_votes)
    });

    HttpResponse::Ok()
        .content_type("application/json")
        .json(votes)
}

#[derive(Deserialize)]
pub struct UiClickRequest {
    pub click_type: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub target: Option<String>,
}

async fn ui_click(req: web::Json<UiClickRequest>, data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    let event = UiClickedEvent {
        event_id: Uuid::new_v4().to_string(),
        click_type: req.click_type.clone(),
        x: req.x,
        y: req.y,
        target: req.target.clone(),
        clicked_at: chrono::Utc::now().to_rfc3339(),
    };
    append_json_event(&events_path, "ui_clicked", &serde_json::to_value(&event).unwrap());
    
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "event_id": event.event_id,
        "click_type": req.click_type,
        "x": req.x,
        "y": req.y,
        "target": req.target
    }))
}

async fn get_all_events(data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    
    let events = get_all_events_json(&events_path);
    
    let result: Vec<HashMap<String, Value>> = events
        .into_iter()
        .map(|(event_type, event_id, payload, created_at)| {
            let mut event = HashMap::new();
            event.insert("event_type".to_string(), serde_json::json!(event_type));
            event.insert("event_id".to_string(), serde_json::json!(event_id));
            event.insert("payload".to_string(), payload);
            event.insert("created_at".to_string(), serde_json::json!(created_at));
            event
        })
        .collect();

    HttpResponse::Ok().json(result)
}

async fn get_events_by_type_endpoint(data: web::Data<AppState>, event_type: web::Path<String>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    let et = event_type.as_str();
    
    let events = get_all_events_json(&events_path);
    
    let filtered: Vec<HashMap<String, Value>> = events
        .into_iter()
        .filter(|(t, _, _, _)| t == et)
        .map(|(_, event_id, payload, created_at)| {
            let mut event = HashMap::new();
            event.insert("event_id".to_string(), serde_json::json!(event_id));
            event.insert("payload".to_string(), payload);
            event.insert("created_at".to_string(), serde_json::json!(created_at));
            event
        })
        .collect();
    
    HttpResponse::Ok().json(serde_json::json!({
        "event_type": et,
        "events": filtered,
        "count": filtered.len()
    }))
}

async fn reset_and_replay(data: web::Data<AppState>) -> impl Responder {
    clear_cache();
    
    let events_path = data.events_path.lock().unwrap();
    let png_data = render_wall_preview(&events_path);
    save_cached_wall(&png_data);
    
    let current_week = get_current_week(&events_path);
    let current_target = get_current_target(&events_path);
    
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "Cache cleared and wall regenerated from events",
        "week": current_week,
        "current_target": current_target.map(|(x, y)| serde_json::json!({"x": x, "y": y})),
        "cache_cleared": true
    }))
}

async fn advance_week(data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    
    let current_week = get_current_week(&events_path);
    let current_target = get_current_target(&events_path);
    let next_week = current_week + 1;
    
    let coordinate_vote_events = get_coordinate_votes_for_week(&events_path, next_week);
    
    let mut coord_vote_counts: HashMap<(i32, i32), i32> = HashMap::new();
    for event in &coordinate_vote_events {
        *coord_vote_counts.entry((event.x, event.y)).or_insert(0) += 1;
    }
    
    let winner_coord = coord_vote_counts.iter()
        .max_by_key(|(_, count)| *count)
        .map(|((x, y), _)| (*x, *y));
    
    if winner_coord.is_none() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "No coordinate has been voted on yet for next week. Vote for a coordinate first."
        }));
    }
    
    let (next_target_x, next_target_y) = winner_coord.unwrap();
    
    let image_events = get_current_week_images(&events_path, current_week);
    
    let image_vote_events: Vec<ImageVotedEvent> = get_events_by_type(&events_path, "image_voted")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    
    let mut img_vote_counts: HashMap<String, i32> = HashMap::new();
    for vote in &image_vote_events {
        if image_events.iter().any(|e| e.event_id == vote.image_event_id) {
            *img_vote_counts.entry(vote.image_event_id.clone()).or_insert(0) += 1;
        }
    }
    
    let winner_image_id = img_vote_counts.iter()
        .max_by_key(|(_, count)| *count)
        .map(|(id, _)| id.clone());
    
    let (applied_x, applied_y) = current_target.unwrap_or((-1, -1));
    
    let event = WeekAdvancedEvent {
        event_id: Uuid::new_v4().to_string(),
        from_week: current_week,
        to_week: next_week,
        applied_x,
        applied_y,
        winning_image_event_id: winner_image_id,
        next_target_x,
        next_target_y,
        applied_at: chrono::Utc::now().to_rfc3339(),
    };

    append_json_event(&events_path, "week_advanced", &serde_json::to_value(&event).unwrap());

    clear_cache();
    let png_data = render_wall_preview(&events_path);
    save_cached_wall(&png_data);
    let base64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_data);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "week": next_week,
        "winner_coordinate": serde_json::json!({"x": next_target_x, "y": next_target_y}),
        "winner_image": event.winning_image_event_id,
        "wall_image": format!("data:image/png;base64,{}", base64_data)
    }))
}

async fn get_stats(data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    
    let image_vote_events: Vec<ImageVotedEvent> = get_events_by_type(&events_path, "image_voted")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    
    let current_week = get_current_week(&events_path);
    let next_week = current_week + 1;
    
    let coordinate_vote_events = get_coordinate_votes_for_week(&events_path, next_week);
    
    let image_events = get_current_week_images(&events_path, current_week);

    HttpResponse::Ok().json(serde_json::json!({
        "week": current_week,
        "current_target": get_current_target(&events_path).map(|(x, y)| serde_json::json!({"x": x, "y": y})),
        "total_image_votes": image_vote_events.len(),
        "total_coordinate_votes": coordinate_vote_events.len(),
        "total_images": image_events.len(),
        "wall_dimensions": {
            "plates_width": WALL_WIDTH_PLATES,
            "plates_height": WALL_HEIGHT_PLATES,
            "plate_size": PLATE_SIZE,
            "total_width": TOTAL_WIDTH,
            "total_height": TOTAL_HEIGHT
        }
    }))
}

async fn get_week_history(data: web::Data<AppState>) -> impl Responder {
    let events_path = data.events_path.lock().unwrap();
    
    let week_events: Vec<WeekAdvancedEvent> = get_events_by_type(&events_path, "week_advanced")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    
    let image_events: HashMap<String, ImageUploadedEvent> = get_events_by_type::<ImageUploadedEvent>(&events_path, "image_uploaded")
        .into_iter()
        .map(|(_, e)| (e.event_id.clone(), e))
        .collect();

    let history: Vec<Value> = week_events
        .iter()
        .map(|e| {
            let mut result = serde_json::json!({
                "week": e.to_week,
                "target_x": e.next_target_x,
                "target_y": e.next_target_y,
            });
            
            if e.applied_x >= 0 {
                result["applied_x"] = serde_json::json!(e.applied_x);
                result["applied_y"] = serde_json::json!(e.applied_y);
            }
            
            if let Some(ref img_id) = e.winning_image_event_id {
                result["winning_image_id"] = serde_json::json!(img_id);
                if let Some(img) = image_events.get(img_id) {
                    result["winning_image_filename"] = serde_json::json!(&img.filename);
                }
            }
            
            result["applied_at"] = serde_json::json!(&e.applied_at);
            
            result
        })
        .collect();

    HttpResponse::Ok()
        .content_type("application/json")
        .json(history)
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
        .stats { display: flex; gap: 20px; flex-wrap: wrap; }
        .stat-box { background: #e3f2fd; padding: 15px; border-radius: 4px; text-align: center; }
        .stat-box .value { font-size: 24px; font-weight: bold; color: #1976D2; }
        .stat-box .label { color: #666; }
        .approved-colors { display: flex; gap: 10px; flex-wrap: wrap; margin: 10px 0; }
        .color-swatch { width: 30px; height: 30px; border: 1px solid #333; border-radius: 4px; }
        .wall-container { text-align: center; }
        .wall-instruction { font-style: italic; color: #666; margin-bottom: 10px; }
        .wall-wrapper { position: relative; display: inline-block; }
        .wall-image { display: block; }
        .wall-overlay { position: absolute; top: 0; left: 0; width: 810px; height: 360px; display: grid; grid-template-columns: repeat(27, 30px); grid-template-rows: repeat(12, 30px); pointer-events: none; opacity: 0; transition: opacity 0.2s; }
        .wall-wrapper:hover .wall-overlay { opacity: 1; pointer-events: auto; }
        .overlay-cell { width: 26px; height: 26px; border: 2px solid rgba(0,0,0,0.3); cursor: pointer; font-size: 10px; font-family: monospace; background: rgba(255,255,255,0.3); transition: background 0.1s; display: flex; align-items: center; justify-content: center; }
        .overlay-cell:hover { background: rgba(76, 175, 80, 0.8); }
        .overlay-cell.current-target { width: 26px; height: 26px; outline: 4px solid rgba(245, 124, 0, 1); animation: pulse 1s ease-in-out infinite; }
        .overlay-cell.current-target:hover { background: rgba(76, 175, 80, 0.8); }
        .overlay-cell.has-votes { background: rgba(200, 230, 201, 0.7); }
        @keyframes pulse {
            0%, 100% { opacity: 1; }
            50% { opacity: 0.3; }
        }
        .week-nav { display: flex; align-items: center; justify-content: center; gap: 15px; margin: 10px 0; }
        .week-nav button { background: #9c27b0; color: white; border: none; padding: 10px 20px; cursor: pointer; border-radius: 4px; font-size: 18px; }
        .week-nav button:hover { background: #7b1fa2; }
        .week-nav button:disabled { background: #ccc; cursor: not-allowed; }
        .week-nav span { font-size: 18px; font-weight: bold; }
        .week-info { background: #f3e5f5; padding: 10px; border-radius: 4px; margin-top: 10px; text-align: center; }
        .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 15px; }
        .image-card { border: 1px solid #ddd; padding: 10px; border-radius: 4px; background: #fafafa; }
        .image-card img { width: 100%; height: 150px; object-fit: contain; background: #eee; }
        .image-card h4 { margin: 10px 0 5px; font-size: 14px; word-break: break-all; }
        .image-card p { margin: 5px 0; color: #666; }
        .vote-btn { background: #4CAF50; color: white; border: none; padding: 10px 20px; cursor: pointer; border-radius: 4px; width: 100%; }
        .vote-btn:hover { background: #45a049; }
        .admin-btn { background: #ff9800; color: white; border: none; padding: 15px 30px; cursor: pointer; border-radius: 4px; font-size: 16px; }
        .admin-btn:hover { background: #f57c00; }
        form { margin: 10px 0; }
        input[type="file"] { margin: 10px 0; }
        input[type="submit"] { background: #2196F3; color: white; border: none; padding: 10px 20px; cursor: pointer; border-radius: 4px; }
        input[type="submit"]:hover { background: #1976D2; }
        .coord-grid { display: grid; grid-template-columns: repeat(27, 1fr); gap: 1px; max-width: 100%; }
        .coord-cell { aspect-ratio: 1; border: 1px solid #ccc; cursor: pointer; font-size: 8px; display: flex; align-items: center; justify-content: center; background: white; font-family: monospace; }
        .coord-cell:hover { background: #e0e0e0; }
        .coord-cell.target { background: #ffeb3b; border: 2px solid #f57c00; }
        .coord-cell.has-votes { background: #c8e6c9; }
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
        <h2>Väggen</h2>
        <p class="wall-instruction">Klicka på bilden för att rösta på en ruta att ändra nästa vecka</p>
        <p>Nuvarande mål: <span id="currentTarget">Ingen koordinat vald</span></p>
        <div class="week-nav">
            <button id="prevWeek" onclick="prevWeek()">←</button>
            <span>Vecka <span id="viewingWeek">0</span></span>
            <button id="nextWeek" onclick="nextWeek()">→</button>
        </div>
        <div id="weekHistoryInfo" class="week-info" style="display: none;"></div>
        <div class="wall-wrapper">
            <img id="wallImage" class="wall-image" src="" alt="Väggen" onclick="handleWallClick(event)" onmousemove="handleWallHover(event)" onmouseleave="handleWallLeave()">
            <div id="wallOverlay" class="wall-overlay"></div>
        </div>
    </div>

    <div class="section" id="votingOptionsSection" style="display: none;">
        <h2>Röstningsalternativ för veckan</h2>
        <p>Koordinat att uppdatera: (<span id="votingTargetX">-</span>, <span id="votingTargetY">-</span>)</p>
        <p>Hovra över bilderna för att se hur väggen hade sett ut:</p>
        <div class="grid" id="votingOptionsGrid"></div>
    </div>

    <div class="section" id="coordSection" style="display: none;">
        <h2>Rösta på koordinat (plattor)</h2>
        <p>Klicka på en platta (27x12) för att rösta på vilken koordinat som ska uppdateras härnäst:</p>
        <div class="coord-grid" id="coordGrid"></div>
    </div>

    <div class="section">
        <h2>Rösta på bild</h2>
        <p>Välj vilken bild som ska läggas upp på den valda koordinaten:</p>
        <div class="grid" id="imageGrid"></div>
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
            
            await loadWeekHistory(data.week);
            await loadCoordinateVotes();
        }

        async function loadWeekHistory(currentWeek) {
            const res = await fetch('/api/week-history');
            const history = await res.json();
            window.weekHistory = history;
            window.currentWeek = currentWeek;
            
            updateWeekNavigation();
        }

        function updateWeekNavigation() {
            const history = window.weekHistory || [];
            const currentWeek = window.currentWeek || 0;
            const actualWeek = parseInt(document.getElementById('week').textContent);
            
            document.getElementById('viewingWeek').textContent = currentWeek;
            
            const prevBtn = document.getElementById('prevWeek');
            const nextBtn = document.getElementById('nextWeek');
            
            prevBtn.disabled = currentWeek <= 0;
            nextBtn.disabled = currentWeek >= actualWeek;
            
            const weekInfo = document.getElementById('weekHistoryInfo');
            const historyEntry = history.find(h => h.week === currentWeek);
            
            if (historyEntry) {
                weekInfo.style.display = 'block';
                let infoText = `<strong>Vecka ${historyEntry.week}:</strong> `;
                if (historyEntry.applied_x !== undefined) {
                    infoText += `Bild "${historyEntry.winning_image_filename || '?'}" applicerades på platta (${historyEntry.applied_x}, ${historyEntry.applied_y}). `;
                }
                infoText += `Nästa mål: (${historyEntry.target_x}, ${historyEntry.target_y})`;
                weekInfo.innerHTML = infoText;
                
                const votingSection = document.getElementById('votingOptionsSection');
                document.getElementById('votingTargetX').textContent = historyEntry.target_x;
                document.getElementById('votingTargetY').textContent = historyEntry.target_y;
                votingSection.style.display = 'block';
                
                loadVotingOptionsForWeek(currentWeek, historyEntry.target_x, historyEntry.target_y);
            } else if (currentWeek > 0) {
                weekInfo.style.display = 'block';
                weekInfo.innerHTML = `<strong>Vecka ${currentWeek}:</strong> Ingen historik tillgänglig`;
                document.getElementById('votingOptionsSection').style.display = 'none';
            } else {
                weekInfo.style.display = 'none';
                document.getElementById('votingOptionsSection').style.display = 'none';
            }
            
            loadWallForWeek(currentWeek);
        }
        
        async function loadVotingOptionsForWeek(week, targetX, targetY) {
            const res = await fetch('/api/images/' + week);
            const images = await res.json();
            const grid = document.getElementById('votingOptionsGrid');
            window.votingTarget = { x: targetX, y: targetY };
            window.votingImages = images;
            
            if (images.length === 0) {
                grid.innerHTML = '<p>Inga bilder uppladdade för denna vecka.</p>';
                return;
            }
            
            grid.innerHTML = images.map(img => `
                <div class="image-card" onmouseenter="previewImageOnWallByCoords('${img.id}', ${img.plate_x}, ${img.plate_y})" onmouseleave="resetWallPreview()">
                    <img src="${img.preview}" alt="${img.filename}">
                    <h4>${img.filename}</h4>
                    <p>Position: (${img.plate_x}, ${img.plate_y})</p>
                    <p>Röster: <strong>${img.votes}</strong></p>
                </div>
            `).join('');
        }
        
        function previewImageOnWallByCoords(imageId, plateX, plateY) {
            const img = window.votingImages.find(i => i.id === imageId);
            if (!img) return;
            
            fetch('/api/wall/' + window.currentWeek)
                .then(r => r.json())
                .then(data => {
                    const imgElem = document.getElementById('wallImage');
                    const canvas = document.createElement('canvas');
                    canvas.width = 810;
                    canvas.height = 360;
                    const ctx = canvas.getContext('2d');
                    
                    const wallImg = new Image();
                    wallImg.onload = () => {
                        ctx.drawImage(wallImg, 0, 0);
                        
                        const previewImg = new Image();
                        previewImg.onload = () => {
                            ctx.drawImage(previewImg, plateX * 30, plateY * 30);
                            imgElem.src = canvas.toDataURL();
                        };
                        previewImg.src = img.preview;
                    };
                    wallImg.src = data.image;
                });
        }
        
        async function loadWallForWeek(week) {
            const res = await fetch('/api/wall/' + week);
            const data = await res.json();
            document.getElementById('wallImage').src = data.image;
        }

        function prevWeek() {
            const history = window.weekHistory || [];
            const currentWeek = window.currentWeek;
            if (currentWeek > 0) {
                window.currentWeek--;
                updateWeekNavigation();
            }
        }

        function nextWeek() {
            const actualWeek = parseInt(document.getElementById('week').textContent);
            if (window.currentWeek < actualWeek) {
                window.currentWeek++;
                updateWeekNavigation();
            }
        }

        async function loadWallForWeek(week) {
            const res = await fetch('/api/wall/' + week);
            const data = await res.json();
            document.getElementById('wallImage').src = data.image;
            loadCoordinateVotesForWeek(week);
        }

        async function loadCoordinateVotesForWeek(week) {
            const nextWeek = week + 1;
            const res = await fetch('/api/coordinates/votes');
            const votes = await res.json();
            const voteMap = {};
            votes.forEach(v => { voteMap[`${v.x},${v.y}`] = v.votes; });
            
            let target = null;
            const history = window.weekHistory || [];
            const historyEntry = history.find(h => h.week === week);
            if (historyEntry) {
                target = { x: historyEntry.target_x, y: historyEntry.target_y };
            } else {
                const wallRes = await fetch('/api/wall');
                const wallData = await wallRes.json();
                target = wallData.current_target;
            }
            
            window.currentTarget = target;
            window.voteMap = voteMap;
            
            const overlay = document.getElementById('wallOverlay');
            let html = '';
            for (let y = 0; y < 12; y++) {
                for (let x = 0; x < 27; x++) {
                    const key = `${x},${y}`;
                    const isTarget = target && target.x === x && target.y === y;
                    const votes = voteMap[key] || 0;
                    let classes = 'overlay-cell';
                    if (isTarget) classes += ' current-target';
                    if (votes > 0 && !isTarget) classes += ' has-votes';
                    html += `<div class="${classes}" 
                        onclick="voteCoordinate(${x}, ${y})" data-x="${x}" data-y="${y}">${votes > 0 ? votes : ''}</div>`;
                }
            }
            overlay.innerHTML = html;
        }

        function nextWeek() {
            const actualWeek = parseInt(document.getElementById('week').textContent);
            if (window.currentWeek < actualWeek) {
                window.currentWeek++;
                updateWeekNavigation();
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
            
            window.availableImages = images;
            
            if (images.length === 0) {
                grid.innerHTML = '<p>Inga bilder uppladdade ännu.</p>';
                return;
            }
            
            grid.innerHTML = images.map(img => `
                <div class="image-card" onmouseenter="previewImageOnWall('${img.id}')" onmouseleave="resetWallPreview()">
                    <img src="${img.preview}" alt="${img.filename}">
                    <h4>${img.filename}</h4>
                    <p>Position: (${img.plate_x}, ${img.plate_y})</p>
                    <p>Röster: <strong>${img.votes}</strong></p>
                    <button class="vote-btn" onclick="voteImage('${img.id}')">Rösta</button>
                </div>
            `).join('');
        }
        
        function previewImageOnWall(imageId) {
            const img = window.availableImages.find(i => i.id === imageId);
            if (!img || !window.currentTarget) return;
            
            const target = window.currentTarget;
            fetch('/api/wall/' + window.currentWeek)
                .then(r => r.json())
                .then(data => {
                    const imgElem = document.getElementById('wallImage');
                    const canvas = document.createElement('canvas');
                    canvas.width = 810;
                    canvas.height = 360;
                    const ctx = canvas.getContext('2d');
                    
                    const wallImg = new Image();
                    wallImg.onload = () => {
                        ctx.drawImage(wallImg, 0, 0);
                        
                        const previewImg = new Image();
                        previewImg.onload = () => {
                            ctx.drawImage(previewImg, target.x * 30, target.y * 30);
                            imgElem.src = canvas.toDataURL();
                        };
                        previewImg.src = img.preview;
                    };
                    wallImg.src = data.image;
                });
        }
        
        function resetWallPreview() {
            loadWallForWeek(window.currentWeek);
        }

        async function loadCoordinateVotes() {
            const res = await fetch('/api/coordinates/votes');
            const votes = await res.json();
            const voteMap = {};
            votes.forEach(v => { voteMap[`${v.x},${v.y}`] = v.votes; });
            
            const wallRes = await fetch('/api/wall');
            const wallData = await wallRes.json();
            const target = wallData.current_target;
            
            window.currentTarget = target;
            window.voteMap = voteMap;
            
            const overlay = document.getElementById('wallOverlay');
            let html = '';
            for (let y = 0; y < 12; y++) {
                for (let x = 0; x < 27; x++) {
                    const key = `${x},${y}`;
                    const isTarget = target && target.x === x && target.y === y;
                    const votes = voteMap[key] || 0;
                    html += `<div class="overlay-cell ${isTarget ? 'current-target' : ''} ${votes > 0 && !isTarget ? 'has-votes' : ''}" 
                        onclick="voteCoordinate(${x}, ${y})" data-x="${x}" data-y="${y}">${votes > 0 ? votes : ''}</div>`;
                }
            }
            overlay.innerHTML = html;
            
            const grid = document.getElementById('coordGrid');
            let gridHtml = '';
            for (let y = 0; y < 12; y++) {
                for (let x = 0; x < 27; x++) {
                    const key = `${x},${y}`;
                    const isTarget = target && target.x === x && target.y === y;
                    const votes = voteMap[key] || 0;
                    gridHtml += `<div class="coord-cell ${isTarget ? 'target' : ''} ${votes > 0 && !isTarget ? 'has-votes' : ''}" 
                        onclick="voteCoordinate(${x}, ${y})" title="Plate (${x}, ${y}): ${votes} röster">${votes > 0 ? votes : ''}</div>`;
                }
            }
            grid.innerHTML = gridHtml;
        }

        function handleWallClick(event) {
            const overlay = document.getElementById('wallOverlay');
            const rect = overlay.getBoundingClientRect();
            const x = event.clientX - rect.left;
            const y = event.clientY - rect.top;
            
            const cellWidth = rect.width / 27;
            const cellHeight = rect.height / 12;
            
            const cellX = Math.floor(x / cellWidth);
            const cellY = Math.floor(y / cellHeight);
            
            if (cellX >= 0 && cellX < 27 && cellY >= 0 && cellY < 12) {
                voteCoordinate(cellX, cellY);
            }
        }

        function handleWallHover(event) {
            const overlay = document.getElementById('wallOverlay');
            const rect = overlay.getBoundingClientRect();
            const x = event.clientX - rect.left;
            const y = event.clientY - rect.top;
            
            const cellWidth = rect.width / 27;
            const cellHeight = rect.height / 12;
            
            const cellX = Math.floor(x / cellWidth);
            const cellY = Math.floor(y / cellHeight);
            
            if (cellX >= 0 && cellX < 27 && cellY >= 0 && cellY < 12) {
                const key = `${cellX},${cellY}`;
                const votes = window.voteMap ? (window.voteMap[key] || 0) : 0;
                overlay.title = `(${cellX}, ${cellY}): ${votes} röster`;
            }
        }

        function handleWallLeave() {
            // Nothing needed, CSS handles opacity
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
            if (data.error) {
                document.getElementById('adminResult').innerHTML = `<p style="color: red;">${data.error}</p>`;
                return;
            }
            document.getElementById('adminResult').innerHTML = `
                <p>Vecka ${data.week} har startat!</p>
                <p>Nästa mål: (${data.winner_coordinate.x}, ${data.winner_coordinate.y})</p>
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
    clear_cache();
    println!("Cache cleared on startup");
    
    let events_path = EVENTS_FILE.to_string();
    
    println!("[DEBUG STARTUP] Dumping all events from {}...", EVENTS_FILE);
    let all_events = get_all_events_json(&events_path);
    for (event_type, event_id, payload, created_at) in &all_events {
        println!("[DEBUG STARTUP] event: type={}, id={}, created_at={}", event_type, event_id, created_at);
        println!("[DEBUG STARTUP]   payload: {}", payload);
    }
    
    let png_data = render_wall_preview(&events_path);
    save_cached_wall(&png_data);
    println!("Wall regenerated from events and cached");

    let app_state = web::Data::new(AppState {
        events_path: Mutex::new(events_path),
    });

    println!("Starting server at http://127.0.0.1:8080");
    println!("Wall dimensions: {}x{} plates ({}x{} pixels)", 
             WALL_WIDTH_PLATES, WALL_HEIGHT_PLATES, TOTAL_WIDTH, TOTAL_HEIGHT);

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .route("/", web::get().to(index))
            .route("/api/wall", web::get().to(get_wall))
            .route("/api/wall/{week}", web::get().to(get_wall_for_week))
            .route("/api/images/{week}", web::get().to(get_images_for_week))
            .route("/api/images", web::get().to(list_images))
            .route("/api/upload", web::post().to(upload_image))
            .route("/api/vote", web::post().to(vote_image))
            .route("/api/coordinate/vote", web::post().to(vote_coordinate))
            .route("/api/coordinates/votes", web::get().to(get_coordinate_votes))
            .route("/api/stats", web::get().to(get_stats))
            .route("/api/admin/advance", web::post().to(advance_week))
            .route("/api/week-history", web::get().to(get_week_history))
            .route("/api/ui/click", web::post().to(ui_click))
            .route("/api/events", web::get().to(get_all_events))
            .route("/api/events/{event_type}", web::get().to(get_events_by_type_endpoint))
            .route("/api/admin/reset-replay", web::post().to(reset_and_replay))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

