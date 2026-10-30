//! Place names offline (S1-04): GPS → town, region and country from a list of places shipped
//! with the app (GeoNames, CC BY 4.0). Every device derives the same names from the same GPS, so
//! nothing is synced.

use serde::{Deserialize, Serialize};
use shoebox_core::model::Gps;
use std::collections::HashMap;
use std::sync::OnceLock;

/// A town or city.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceRow {
    pub lat_e7: i32,
    pub lon_e7: i32,
    pub name: String,
    /// Index into [`GazetteerData::admin1`], or `u16::MAX`.
    pub admin1: u16,
    /// Index into [`GazetteerData::countries`].
    pub country: u16,
    pub population: u32,
}

/// What `places.bin` holds; `cargo xtask gazetteer` makes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GazetteerData {
    pub countries: Vec<String>,
    /// (country index, region name)
    pub admin1: Vec<(u16, String)>,
    pub places: Vec<PlaceRow>,
}

const MAGIC: &[u8; 4] = b"SBGZ";
const VERSION: u8 = 1;

impl GazetteerData {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.push(VERSION);
        out.extend(postcard::to_stdvec(self).expect("gazetteer encodes"));
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 5 || &bytes[..4] != MAGIC || bytes[4] != VERSION {
            return None;
        }
        postcard::from_bytes(&bytes[5..]).ok()
    }
}

/// Within this distance a photo is "in" the nearest town.
pub const CITY_KM: f64 = 25.0;
/// Within this distance of any town a photo gets that town's region and country (beyond it,
/// out at sea or in a wilderness, it gets nothing).
pub const REGION_KM: f64 = 200.0;
/// A big city nearby is named too ("Lisbon" for a photo in Amadora).
const BIG_CITY: u32 = 100_000;
const BIG_CITY_KM: f64 = 15.0;

/// Where a photo is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place<'a> {
    /// The nearest town, then a big city nearby if it is another one.
    pub towns: Vec<&'a str>,
    pub region: Option<&'a str>,
    pub country: &'a str,
}

impl Place<'_> {
    /// One name per line, for the facts index.
    pub fn text(&self) -> String {
        let mut lines: Vec<&str> = self.towns.clone();
        lines.extend(self.region);
        lines.push(self.country);
        let mut seen = Vec::new();
        lines.retain(|l| {
            let new = !seen.contains(l);
            seen.push(*l);
            new
        });
        lines.join("\n")
    }
}

/// The place list with a grid of [`CELL_DEG`] cells for nearest lookups.
pub struct Gazetteer {
    data: GazetteerData,
    grid: HashMap<(i16, i16), Vec<u32>>,
}

/// Grid cell size in degrees (about 28 km of latitude).
const CELL_DEG: f64 = 0.25;
const CELLS_PER_DEG: i32 = 4;

fn cell(lat: f64, lon: f64) -> (i16, i16) {
    (
        (lat / CELL_DEG).floor() as i16,
        (lon / CELL_DEG).floor() as i16,
    )
}

fn deg(e7: i32) -> f64 {
    f64::from(e7) / 1e7
}

/// Great-circle distance in km.
pub fn km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, lo1, la2, lo2) = (
        a.0.to_radians(),
        a.1.to_radians(),
        b.0.to_radians(),
        b.1.to_radians(),
    );
    let h = ((la2 - la1) / 2.0).sin().powi(2)
        + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().min(1.0).asin()
}

impl Gazetteer {
    pub fn new(data: GazetteerData) -> Self {
        let mut grid: HashMap<(i16, i16), Vec<u32>> = HashMap::new();
        for (i, p) in data.places.iter().enumerate() {
            grid.entry(cell(deg(p.lat_e7), deg(p.lon_e7)))
                .or_default()
                .push(i as u32);
        }
        Self { data, grid }
    }

    /// The list built into this crate (`data/places.bin`).
    pub fn bundled() -> &'static Gazetteer {
        static G: OnceLock<Gazetteer> = OnceLock::new();
        G.get_or_init(|| {
            let data = GazetteerData::decode(include_bytes!("../data/places.bin"))
                .expect("the bundled places.bin decodes");
            Gazetteer::new(data)
        })
    }

    pub fn len(&self) -> usize {
        self.data.places.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.places.is_empty()
    }

    /// Places within `radius` km of `at`, with their distances.
    fn within(&self, at: (f64, f64), radius: f64) -> impl Iterator<Item = (&PlaceRow, f64)> {
        let per_cell = 111.0 * CELL_DEG;
        let dlat = (radius / per_cell).ceil() as i32;
        let cos = at.0.to_radians().cos().abs().max(0.01);
        let around = 180 * CELLS_PER_DEG;
        let dlon = ((radius / (per_cell * cos)).ceil() as i32).min(around);
        let (clat, clon) = cell(at.0, at.1);
        let (clat, clon) = (i32::from(clat), i32::from(clon));
        let mut cells = Vec::new();
        for la in clat - dlat..=clat + dlat {
            for lo in clon - dlon..=clon + dlon {
                // Wrap around the antimeridian.
                let lo = (lo + around).rem_euclid(2 * around) - around;
                cells.push((la as i16, lo as i16));
            }
        }
        cells.sort_unstable();
        cells.dedup();
        cells
            .into_iter()
            .filter_map(|c| self.grid.get(&c))
            .flatten()
            .map(|&i| &self.data.places[i as usize])
            .map(move |p| (p, km(at, (deg(p.lat_e7), deg(p.lon_e7)))))
            .filter(move |(_, d)| *d <= radius)
    }

    /// Town, region and country for a GPS position; `None` when no town is within
    /// [`REGION_KM`].
    pub fn nearest(&self, gps: Gps) -> Option<Place<'_>> {
        let at = (deg(gps.lat_e7), deg(gps.lon_e7));
        let mut best: Option<(&PlaceRow, f64)> = None;
        let mut big: Option<(&PlaceRow, f64)> = None;
        // Look close first; widen only when nothing is near.
        for radius in [CITY_KM, REGION_KM] {
            for (p, d) in self.within(at, radius) {
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((p, d));
                }
                if p.population >= BIG_CITY
                    && d <= BIG_CITY_KM
                    && big.is_none_or(|(b, _)| p.population > b.population)
                {
                    big = Some((p, d));
                }
            }
            if best.is_some() {
                break;
            }
        }
        let (p, d) = best?;
        let mut towns = Vec::new();
        if d <= CITY_KM {
            towns.push(p.name.as_str());
            if let Some((b, _)) = big.filter(|(b, _)| b.name != p.name) {
                towns.push(b.name.as_str());
            }
        }
        let region = self
            .data
            .admin1
            .get(usize::from(p.admin1))
            .map(|(_, name)| name.as_str());
        let country = self.data.countries.get(usize::from(p.country))?.as_str();
        Some(Place {
            towns,
            region,
            country,
        })
    }
}
