// swift-tools-version:6.0
// ShoeboxPhotos (P1-03): the one PhotoKit implementation, for the Mac (P1-04, through the C entry
// points in ShoeboxPhotosBridge) and the iPhone (P1-06, a Tauri plugin over ShoeboxPhotos). Only
// Foundation, Photos, AVFoundation and ImageIO: no UIKit or AppKit, so it builds for both.
import PackageDescription

// Swift 5 checking: PhotoKit's callbacks are not annotated for Swift 6 concurrency. A flag, not
// `.swiftLanguageMode(.v5)`: the Command Line Tools 26.5 manifest interface lacks that API.
let swift5: [SwiftSetting] = [.unsafeFlags(["-swift-version", "5"])]

let package = Package(
    name: "ShoeboxPhotos",
    platforms: [.macOS(.v13), .iOS(.v16)],
    products: [
        .library(name: "ShoeboxPhotos", targets: ["ShoeboxPhotos"]),
        // Linked into the Rust app on macOS by `build.rs` (swift-rs).
        .library(name: "ShoeboxPhotosBridge", type: .static, targets: ["ShoeboxPhotosBridge"]),
    ],
    targets: [
        .target(name: "ShoeboxPhotos", swiftSettings: swift5),
        .target(name: "ShoeboxPhotosBridge", dependencies: ["ShoeboxPhotos"], swiftSettings: swift5),
        .testTarget(name: "ShoeboxPhotosTests", dependencies: ["ShoeboxPhotos"]),
    ]
)
