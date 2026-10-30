//! Place names for photos with GPS (S1-04), from the gazetteer bundled in `shoebox-search`.

use shoebox_core::model::Gps;
use shoebox_search::places::Gazetteer;

/// "Town\nRegion\nCountry" for the facts index, or `None` when nothing is near enough.
pub(crate) fn place_text(gps: Gps) -> Option<String> {
    Gazetteer::bundled().nearest(gps).map(|p| p.text())
}
