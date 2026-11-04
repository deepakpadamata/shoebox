// The parts of ShoeboxPhotos that need no Photos library: which resource is "the photo", names,
// MIME types, change tokens and replies. Tested by `swift test` on any Mac.
import Foundation
import UniformTypeIdentifiers

/// The resource kinds Shoebox looks at (`PHAssetResourceType`, without Photos).
public enum ResourceKind: Equatable, Sendable {
    case photo, fullSizePhoto, video, fullSizeVideo, pairedVideo, fullSizePairedVideo, other
}

/// One resource of an asset, as far as choosing and naming go.
public struct ResourceInfo: Equatable, Sendable {
    public var kind: ResourceKind
    /// The name the file had when it came in (`IMG_0001.HEIC`; an edit's is `FullSizeRender.jpg`).
    public var filename: String
    public var uti: String
    /// Bytes, where Photos says.
    public var size: Int64?
    /// Whether the bytes are on this device (`false`: in iCloud only), where Photos says.
    public var local: Bool?

    public init(kind: ResourceKind, filename: String, uti: String, size: Int64? = nil, local: Bool? = nil) {
        self.kind = kind
        self.filename = filename
        self.uti = uti
        self.size = size
        self.local = local
    }
}

public enum Choice {
    /// The resource that is the photo or video (D2): the current version, so the full-size render
    /// when the asset was edited, otherwise the original.
    public static func main(isVideo: Bool, _ rs: [ResourceInfo]) -> Int? {
        let order: [ResourceKind] = isVideo ? [.fullSizeVideo, .video] : [.fullSizePhoto, .photo]
        for k in order { if let i = rs.firstIndex(where: { $0.kind == k }) { return i } }
        return nil
    }

    /// A Live Photo's video, in the same version as its still (P1-08): the edited one when there is.
    public static func paired(_ rs: [ResourceInfo]) -> Int? {
        let edited = rs.contains { $0.kind == .fullSizePhoto }
        let order: [ResourceKind] = edited ? [.fullSizePairedVideo, .pairedVideo] : [.pairedVideo]
        for k in order { if let i = rs.firstIndex(where: { $0.kind == k }) { return i } }
        return nil
    }

    /// The file name the main resource is read into: the original's name (an edit keeps the name
    /// the photo came in with) and the main resource's extension.
    public static func name(_ rs: [ResourceInfo], main: Int) -> String {
        let original = rs.first { $0.kind == .photo || $0.kind == .video }?.filename ?? rs[main].filename
        return named(stem: stem(original), like: rs[main].filename, fallback: "photo")
    }

    /// The paired video's file name: the still's stem, so an import pairs the two as it pairs
    /// `IMG_0001.HEIC` and `IMG_0001.MOV` in a folder (A-109).
    public static func pairedName(still: String, _ video: ResourceInfo) -> String {
        named(stem: stem(still), like: video.filename, fallback: "video")
    }

    static func stem(_ name: String) -> String {
        let s = (name as NSString).deletingPathExtension
        return safe(s)
    }

    static func named(stem: String, like file: String, fallback: String) -> String {
        let ext = safe((file as NSString).pathExtension)
        let base = stem.isEmpty ? fallback : stem
        return ext.isEmpty ? base : "\(base).\(ext)"
    }

    /// No directories, nothing hidden.
    static func safe(_ s: String) -> String {
        var out = s.replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: ":", with: "_")
        while out.hasPrefix(".") { out.removeFirst() }
        return out
    }
}

public enum Mime {
    public static func of(uti: String, filename: String) -> String {
        if let m = UTType(uti)?.preferredMIMEType { return m }
        let ext = (filename as NSString).pathExtension
        if let m = UTType(filenameExtension: ext)?.preferredMIMEType { return m }
        return "application/octet-stream"
    }
}

/// One asset as the Rust side reads it: the shape of the Android plugin's (`PhotoBackend.kt`), with
/// `library` set, since a PhotoKit original is read through the library, not from a path.
public struct AssetReply: Codable, Equatable, Sendable {
    public var id: String
    public var modified_ms: Int64
    public var size: Int64
    public var mime: String
    public var library: Bool = true
    /// The original is in iCloud only ("Optimize Mac Storage").
    public var cloud_only: Bool
    public var name: String
    /// A Live Photo, with a video to keep beside the still.
    public var live: Bool

    public init(id: String, modified_ms: Int64, size: Int64, mime: String, cloud_only: Bool, name: String, live: Bool) {
        self.id = id
        self.modified_ms = modified_ms
        self.size = size
        self.mime = mime
        self.cloud_only = cloud_only
        self.name = name
        self.live = live
    }

    /// The asset with resources `rs`, or nil when none of them is a photo or a video.
    public static func of(id: String, modified: Date?, isVideo: Bool, _ rs: [ResourceInfo]) -> AssetReply? {
        guard let i = Choice.main(isVideo: isVideo, rs) else { return nil }
        let m = rs[i]
        return AssetReply(
            id: id,
            modified_ms: Int64(((modified ?? Date(timeIntervalSince1970: 0)).timeIntervalSince1970 * 1000).rounded()),
            size: m.size ?? 0,
            mime: Mime.of(uti: m.uti, filename: m.filename),
            // Unknown counts as here: a read without the network then says otherwise.
            cloud_only: m.local == false,
            name: Choice.name(rs, main: i),
            live: !isVideo && Choice.paired(rs) != nil
        )
    }
}

public struct ScanReply: Codable, Equatable, Sendable {
    public var assets: [AssetReply]
    public var removed: [String]
    public var token: String
    /// Everything, not only changes: no token was given, or it could not be used.
    public var full: Bool
}

/// What a read wrote: the photo, and a Live Photo's video beside it.
public struct ReadReply: Codable, Equatable, Sendable {
    public var path: String
    public var video: String?
}

/// Change tokens as strings the Rust side keeps: a versioned base64 of the archived token.
public enum Token {
    static let prefix = "pk1:"

    public static func encode(_ archived: Data) -> String { prefix + archived.base64EncodedString() }

    /// The archived token, or nil when there is none or it is not one of ours (a full scan follows).
    public static func decode(_ s: String?) -> Data? {
        guard let s, s.hasPrefix(prefix) else { return nil }
        return Data(base64Encoded: String(s.dropFirst(prefix.count)))
    }
}

/// The net effect of change batches in order: ids to look up again, and ids gone.
public struct ChangeSet: Equatable, Sendable {
    public var changed: [String] = []
    public var deleted: [String] = []

    public init() {}

    public static func merge(_ batches: [(inserted: [String], updated: [String], deleted: [String])]) -> ChangeSet {
        var last: [String: Bool] = [:]  // id → still there
        var order: [String] = []
        for b in batches {
            for id in b.inserted + b.updated {
                if last[id] == nil { order.append(id) }
                last[id] = true
            }
            for id in b.deleted {
                if last[id] == nil { order.append(id) }
                last[id] = false
            }
        }
        var out = ChangeSet()
        for id in order {
            if last[id] == true { out.changed.append(id) } else { out.deleted.append(id) }
        }
        return out
    }
}
