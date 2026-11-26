//! Resolve JOAAT hashes by brute-forcing against name databases.
//!
//! Usage: forge9-resolve-hashes --targets hashes.txt --names names.tsv [--wordlist extra.txt]
//!
//! Reads target hashes (hex, one per line), tries every name from the TSV/wordlist
//! in both case-sensitive and lowercase JOAAT, and prints matches as `hash name`
//! lines (compatible with forge9 --dict).
//!
//! The names.tsv format (rage-tools-vi): tab-separated, header row, column 0 = hash, column 1 = name.
//! Wordlist format: one candidate name per line.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, BufRead};

fn joaat(s: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in s {
        h = h.wrapping_add(b as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut targets_path = None;
    let mut name_paths: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--targets" => { i += 1; targets_path = Some(args[i].clone()); }
            "--names" | "--wordlist" => { i += 1; name_paths.push(args[i].clone()); }
            _ => { name_paths.push(args[i].clone()); }
        }
        i += 1;
    }

    // Load targets
    let targets: HashSet<u32> = match targets_path {
        Some(p) => fs::read_to_string(&p).expect("read targets")
            .lines()
            .filter_map(|l| {
                let l = l.trim().trim_start_matches("0x").trim_start_matches("0X");
                u32::from_str_radix(l, 16).ok()
            })
            .collect(),
        None => {
            // Read from stdin
            io::stdin().lock().lines()
                .filter_map(|l| l.ok())
                .filter_map(|l| {
                    let l = l.trim().trim_start_matches("0x").trim_start_matches("0X").to_string();
                    u32::from_str_radix(&l, 16).ok()
                })
                .collect()
        }
    };

    if targets.is_empty() {
        eprintln!("no target hashes loaded");
        return;
    }
    eprintln!("{} target hashes loaded", targets.len());

    let mut found = 0usize;
    let mut seen = HashSet::new();

    // ponytail: prefixes to try. Common RAGE engine prefixes.
    let prefixes: &[&str] = &["", "rage__", "rage__fw", "CW_", "rage__fwui", "C", "c"];

    let try_name = |name: &str, targets: &HashSet<u32>, seen: &mut HashSet<u32>, found: &mut usize| {
        // Case-sensitive (meta fields)
        let h = joaat(name.as_bytes());
        if targets.contains(&h) && seen.insert(h) {
            println!("{h:#010x} {name}");
            *found += 1;
        }
        // Lowercase (shader/texture names)
        let lower = name.to_lowercase();
        if lower != name {
            let hl = joaat(lower.as_bytes());
            if targets.contains(&hl) && seen.insert(hl) {
                println!("{hl:#010x} {lower}");
                *found += 1;
            }
        }
    };

    for path in &name_paths {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));

        if path.ends_with(".tsv") {
            // rage-tools-vi format: hash\tname\tkind\t...
            for line in text.lines().skip(1) {
                let name = match line.split('\t').nth(1) {
                    Some(n) if !n.is_empty() => n,
                    _ => continue,
                };
                // Try the name as-is and with prefixes
                try_name(name, &targets, &mut seen, &mut found);
                for pfx in prefixes.iter().skip(1) {
                    let prefixed = format!("{pfx}{name}");
                    try_name(&prefixed, &targets, &mut seen, &mut found);
                }
            }
        } else {
            // Plain wordlist: one name per line
            for line in text.lines() {
                let name = line.trim();
                if name.is_empty() || name.starts_with('#') { continue; }
                try_name(name, &targets, &mut seen, &mut found);
                for pfx in prefixes.iter().skip(1) {
                    let prefixed = format!("{pfx}{name}");
                    try_name(&prefixed, &targets, &mut seen, &mut found);
                }
            }
        }
    }

    // Built-in game dev terms (covers common DOM tags/attributes)
    let builtins = [
        // Core
        "root", "node", "child", "parent", "sibling", "index", "count", "size", "offset",
        "value", "hash", "id", "key", "item", "entry", "data", "type", "name", "flags",
        "enabled", "disabled", "visible", "hidden", "active", "locked", "default",
        // Transform
        "position", "rotation", "scale", "transform", "matrix", "origin", "pivot",
        "x", "y", "z", "w", "roll", "pitch", "yaw", "heading", "forward", "up", "right",
        // Spatial
        "min", "max", "radius", "extent", "bounds", "aabb", "sphere", "center",
        "width", "height", "depth", "length", "distance", "range", "near", "far",
        // Game
        "health", "damage", "armor", "speed", "velocity", "acceleration", "mass",
        "weapon", "ammo", "vehicle", "ped", "player", "camera", "audio", "animation",
        "spawn", "mission", "objective", "reward", "score", "level", "difficulty",
        "time", "duration", "delay", "interval", "timer", "cooldown", "frequency",
        // World
        "weather", "wind", "fog", "rain", "snow", "cloud", "sun", "moon", "sky",
        "water", "ocean", "river", "lake", "wave", "current", "tide",
        "terrain", "ground", "road", "path", "route", "waypoint", "zone", "area", "region",
        // AI
        "behavior", "behaviour", "state", "task", "action", "event", "trigger", "condition",
        "target", "threat", "ally", "enemy", "neutral", "hostile", "friendly",
        "patrol", "guard", "flee", "chase", "attack", "search", "investigate",
        // UI
        "widget", "panel", "button", "label", "text", "image", "icon", "color", "alpha",
        "font", "alignment", "padding", "margin", "border", "background", "foreground",
        "tooltip", "cursor", "focus", "hover", "pressed", "selected", "highlighted",
        // Vehicle
        "engine", "wheel", "tire", "suspension", "brake", "steering", "gear", "throttle",
        "fuel", "nitro", "boost", "drift", "traction", "handling", "torque", "rpm",
        // Audio
        "volume", "pitch", "pan", "loop", "fade", "mute", "reverb", "echo", "delay",
        "sound", "music", "voice", "ambient", "effect", "stream", "sample",
        // Network
        "session", "lobby", "network", "sync", "replicate", "client", "server", "host",
        "player_list", "squad", "team", "online", "multiplayer", "matchmaking",
        // Clothing/Ped
        "component", "drawable", "texture", "variation", "palette", "tint", "material",
        "diffuse", "normal", "specular", "emissive", "detail", "mask", "overlay",
        "head", "body", "hair", "eyes", "legs", "feet", "hands", "torso", "upper", "lower",
        // Profile/Tag (from resolved DOM names)
        "profile", "tag", "tags", "points", "priority", "fov",
        // GTA-specific
        "CMapData", "CEntityDef", "CBaseArchetypeDef", "CTimeCycleModifier",
        "CExtensionDefExpression", "CExtensionDefLightEffect", "rage__phVerletClothCustomBounds",
        "archetype", "archetypeName", "entityDef", "extensionDef", "timecycleModifier",
        "contentFlags", "lodDist", "childLodDist", "numChildren", "parentIndex",
        "streamingExtentsMin", "streamingExtentsMax", "entitiesExtentsMin", "entitiesExtentsMax",
    ];
    for name in &builtins {
        try_name(name, &targets, &mut seen, &mut found);
        // Also PascalCase and camelCase
        if name.contains('_') {
            let pascal: String = name.split('_').map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(ch) => ch.to_uppercase().collect::<String>() + &c.collect::<String>(),
                    None => String::new(),
                }
            }).collect();
            try_name(&pascal, &targets, &mut seen, &mut found);
        }
    }

    eprintln!("{found} hashes resolved out of {}", targets.len());
}
