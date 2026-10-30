//! S1-04: GPS → town, region and country with the bundled GeoNames list.

use shoebox_core::model::Gps;
use shoebox_search::places::Gazetteer;
use std::time::Instant;

fn gps(lat: f64, lon: f64) -> Gps {
    Gps {
        lat_e7: (lat * 1e7) as i32,
        lon_e7: (lon * 1e7) as i32,
    }
}

/// (where, lat, lon, a town named, region, country)
const POINTS: &[(&str, f64, f64, &str, &str, &str)] = &[
    (
        "Lisbon, Praça do Comércio",
        38.7075,
        -9.1364,
        "Lisbon",
        "Lisbon",
        "Portugal",
    ),
    ("Kochi, Kerala", 9.9312, 76.2673, "Kochi", "Kerala", "India"),
    (
        "Paris, Louvre",
        48.8606,
        2.3376,
        "Paris",
        "Île-de-France",
        "France",
    ),
    (
        "Manhattan",
        40.7580,
        -73.9855,
        "New York City",
        "New York",
        "United States",
    ),
    (
        "Tokyo, Shibuya",
        35.6595,
        139.7005,
        "Shibuya",
        "Tokyo",
        "Japan",
    ),
    (
        "Sydney Opera House",
        -33.8568,
        151.2153,
        "Sydney",
        "New South Wales",
        "Australia",
    ),
    (
        "Cape Town",
        -33.9249,
        18.4241,
        "Cape Town",
        "Western Cape",
        "South Africa",
    ),
    (
        "Rio de Janeiro",
        -22.9068,
        -43.1729,
        "Rio de Janeiro",
        "Rio de Janeiro",
        "Brazil",
    ),
    (
        "Reykjavík",
        64.1466,
        -21.9426,
        "Reykjavík",
        "Capital Region",
        "Iceland",
    ),
    (
        "Honolulu, Hawaii",
        21.3069,
        -157.8583,
        "Honolulu",
        "Hawaii",
        "United States",
    ),
    (
        "Funchal, Madeira (island)",
        32.6669,
        -16.9241,
        "Funchal",
        "Madeira",
        "Portugal",
    ),
    (
        "Valletta, Malta (island)",
        35.8989,
        14.5146,
        "Valletta",
        "",
        "Malta",
    ),
    (
        "Strasbourg (French side of the Rhine)",
        48.5734,
        7.7521,
        "Strasbourg",
        "Grand Est",
        "France",
    ),
    (
        "Kehl (German side of the Rhine)",
        48.5729,
        7.8150,
        "Kehl",
        "Baden-Wurttemberg",
        "Germany",
    ),
    (
        "El Paso (US side of the border)",
        31.7619,
        -106.4850,
        "El Paso",
        "Texas",
        "United States",
    ),
    (
        "Ciudad Juárez (Mexican side)",
        31.6904,
        -106.4245,
        "Ciudad Juárez",
        "Chihuahua",
        "Mexico",
    ),
    (
        "Nairobi",
        -1.2921,
        36.8219,
        "Nairobi",
        "Nairobi County",
        "Kenya",
    ),
    (
        "Ushuaia",
        -54.8019,
        -68.3030,
        "Ushuaia",
        "Tierra del Fuego",
        "Argentina",
    ),
    (
        "Suva, Fiji (near the antimeridian)",
        -18.1416,
        178.4419,
        "Suva",
        "Central",
        "Fiji",
    ),
    ("Singapore", 1.2903, 103.8520, "Singapore", "", "Singapore"),
];

#[test]
fn hand_picked_points() {
    let g = Gazetteer::bundled();
    let mut bad = vec![];
    for (what, lat, lon, town, region, country) in POINTS {
        let Some(p) = g.nearest(gps(*lat, *lon)) else {
            bad.push(format!("{what}: nothing"));
            continue;
        };
        let ok_town = p.towns.contains(town);
        let ok_region = region.is_empty() || p.region == Some(*region);
        if !ok_town || !ok_region || p.country != *country {
            bad.push(format!(
                "{what}: {:?} / {:?} / {}",
                p.towns, p.region, p.country
            ));
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

#[test]
fn far_from_anything_is_nothing() {
    let g = Gazetteer::bundled();
    assert!(g.nearest(gps(-40.0, -130.0)).is_none(), "the South Pacific");
    // Out at sea but within reach of a coast: no town, the region of the nearest one.
    let p = g.nearest(gps(38.5, -9.8)).expect("off Lisbon");
    assert!(p.towns.is_empty());
    assert_eq!(p.country, "Portugal");
}

#[test]
fn hundred_thousand_lookups() {
    let g = Gazetteer::bundled();
    let t = Instant::now();
    let mut found = 0;
    for i in 0..100_000u32 {
        // Spread over Europe and India.
        let (lat, lon) = if i % 2 == 0 {
            (
                36.0 + f64::from(i % 1000) * 0.02,
                -9.0 + f64::from(i % 997) * 0.03,
            )
        } else {
            (
                8.0 + f64::from(i % 1000) * 0.02,
                72.0 + f64::from(i % 991) * 0.02,
            )
        };
        found += usize::from(g.nearest(gps(lat, lon)).is_some());
    }
    assert!(found > 50_000, "{found} found");
    // Under 1 s on a Pi 5 (S1-04) in release; debug builds are slower.
    let budget = if cfg!(debug_assertions) { 20 } else { 1 };
    assert!(t.elapsed().as_secs() < budget, "{:?}", t.elapsed());
}

#[test]
fn the_list_is_small() {
    assert!(include_bytes!("../data/places.bin").len() <= 5 << 20);
    assert!(Gazetteer::bundled().len() > 20_000);
}
