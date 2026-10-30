//! `cargo xtask gazetteer`: build `crates/shoebox-search/data/places.bin` from GeoNames dumps
//! (S1-04). The inputs are downloaded by hand (this task never fetches) from
//! <https://download.geonames.org/export/dump/>: `cities5000.zip` (unzipped), `admin1CodesASCII.txt`
//! and `countryInfo.txt`. GeoNames is CC BY 4.0; the attribution is in each app's notices.

use shoebox_search::places::{GazetteerData, PlaceRow};
use std::collections::HashMap;
use std::path::Path;

pub struct Summary {
    pub places: usize,
    pub regions: usize,
    pub countries: usize,
    pub bytes: usize,
}

fn read(dir: &Path, name: &str) -> Result<String, String> {
    std::fs::read_to_string(dir.join(name))
        .map_err(|e| format!("{}: {e}", dir.join(name).display()))
}

/// Parse the three GeoNames files in `dir`.
pub fn build(dir: &Path) -> Result<GazetteerData, String> {
    let mut data = GazetteerData::default();
    // countryInfo.txt: ISO, ISO3, ISO-numeric, fips, Country, …; comments start with '#'.
    let mut country_ix: HashMap<String, u16> = HashMap::new();
    for line in read(dir, "countryInfo.txt")?
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() > 4 && !f[0].is_empty() {
            country_ix.insert(f[0].to_string(), data.countries.len() as u16);
            data.countries.push(f[4].to_string());
        }
    }
    // admin1CodesASCII.txt: "US.CA", name, ascii name, geonameid.
    let mut admin_ix: HashMap<String, u16> = HashMap::new();
    for line in read(dir, "admin1CodesASCII.txt")?.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let Some((cc, _)) = f[0].split_once('.') else {
            continue;
        };
        let Some(&c) = country_ix.get(cc) else {
            continue;
        };
        if f.len() > 1 {
            admin_ix.insert(f[0].to_string(), data.admin1.len() as u16);
            data.admin1.push((c, f[1].to_string()));
        }
    }
    // cities: id, name, ascii, alternates, lat, lon, class, code, country, cc2, admin1, …, population (14).
    let cities = ["cities5000.txt", "cities15000.txt"]
        .iter()
        .find_map(|n| read(dir, n).ok())
        .ok_or("neither cities5000.txt nor cities15000.txt is there")?;
    for line in cities.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 15 {
            continue;
        }
        let Some(&country) = country_ix.get(f[8]) else {
            continue;
        };
        let (Ok(lat), Ok(lon)) = (f[4].parse::<f64>(), f[5].parse::<f64>()) else {
            continue;
        };
        let admin1 = admin_ix
            .get(&format!("{}.{}", f[8], f[10]))
            .copied()
            .unwrap_or(u16::MAX);
        data.places.push(PlaceRow {
            lat_e7: (lat * 1e7).round() as i32,
            lon_e7: (lon * 1e7).round() as i32,
            name: f[1].to_string(),
            admin1,
            country,
            population: f[14].parse().unwrap_or(0),
        });
    }
    // Deterministic output: by position.
    data.places
        .sort_by_key(|p| (p.lat_e7, p.lon_e7, p.name.clone()));
    Ok(data)
}

pub fn write(dir: &Path, out: &Path) -> Result<Summary, String> {
    let data = build(dir)?;
    let bytes = data.encode();
    std::fs::write(out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(Summary {
        places: data.places.len(),
        regions: data.admin1.len(),
        countries: data.countries.len(),
        bytes: bytes.len(),
    })
}
