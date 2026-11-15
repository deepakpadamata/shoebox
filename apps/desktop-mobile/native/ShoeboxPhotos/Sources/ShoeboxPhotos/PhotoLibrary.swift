// The system Photos library through PhotoKit (P1-03). Every call blocks until PhotoKit answers, so
// call them from a background thread, never the main one.
import AVFoundation
import Foundation
import ImageIO
import Photos
import UniformTypeIdentifiers

public struct PhotosError: Error, CustomStringConvertible {
    public let description: String
    init(_ d: String) { description = d }
}

public enum PhotoLibrary {
    /// `granted`, `limited`, `denied` (also when restricted) or `undetermined` (never asked).
    public static func permission() -> String {
        name(PHPhotoLibrary.authorizationStatus(for: .readWrite))
    }

    /// Ask the person, if they have not been asked yet; the answer.
    public static func requestPermission() -> String {
        let done = DispatchSemaphore(value: 0)
        var answer = PHAuthorizationStatus.notDetermined
        PHPhotoLibrary.requestAuthorization(for: .readWrite) { s in
            answer = s
            done.signal()
        }
        done.wait()
        return name(answer)
    }

    static func name(_ s: PHAuthorizationStatus) -> String {
        switch s {
        case .authorized: return "granted"
        case .limited: return "limited"
        case .notDetermined: return "undetermined"
        default: return "denied"
        }
    }

    static func requireAccess() throws {
        let s = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        guard s == .authorized || s == .limited else { throw PhotosError("access to Photos was not granted") }
    }

    /// The person's own photos: not shared albums (other people's) or hidden ones.
    static func options() -> PHFetchOptions {
        let o = PHFetchOptions()
        o.includeAssetSourceTypes = [.typeUserLibrary, .typeiTunesSynced]
        o.includeHiddenAssets = false
        return o
    }

    /// What changed since `token`, or everything when there is none or it can no longer be used.
    /// The next token is taken before a full walk, so a change during it is reported again.
    public static func scan(token: String?) throws -> ScanReply {
        try requireAccess()
        let lib = PHPhotoLibrary.shared()
        // With only some photos shared the set is small, and what the person adds to it is not a
        // change PhotoKit reports: look at all of it every time.
        let limited = PHPhotoLibrary.authorizationStatus(for: .readWrite) == .limited
        if !limited, let data = Token.decode(token),
           let since = try? NSKeyedUnarchiver.unarchivedObject(ofClass: PHPersistentChangeToken.self, from: data),
           let reply = try? changes(lib, since: since)
        {
            return reply
        }
        let next = try archive(lib.currentChangeToken)
        var assets: [AssetReply] = []
        PHAsset.fetchAssets(with: options()).enumerateObjects { a, _, _ in
            if let r = reply(a) { assets.append(r) }
        }
        return ScanReply(assets: assets, removed: [], token: next, full: true)
    }

    static func changes(_ lib: PHPhotoLibrary, since: PHPersistentChangeToken) throws -> ScanReply {
        var batches: [(inserted: [String], updated: [String], deleted: [String])] = []
        var last = since
        for c in try lib.fetchPersistentChanges(since: since) {
            let d = try c.changeDetails(for: .asset)
            batches.append((Array(d.insertedLocalIdentifiers), Array(d.updatedLocalIdentifiers), Array(d.deletedLocalIdentifiers)))
            last = c.changeToken
        }
        let set = ChangeSet.merge(batches)
        var assets: [AssetReply] = []
        if !set.changed.isEmpty {
            // One that is no longer found (hidden, moved to a shared album) is not reported removed:
            // only a deletion is.
            PHAsset.fetchAssets(withLocalIdentifiers: set.changed, options: options()).enumerateObjects { a, _, _ in
                if let r = reply(a) { assets.append(r) }
            }
        }
        return ScanReply(assets: assets, removed: set.deleted, token: try archive(last), full: false)
    }

    static func archive(_ t: PHPersistentChangeToken) throws -> String {
        Token.encode(try NSKeyedArchiver.archivedData(withRootObject: t, requiringSecureCoding: true))
    }

    static func reply(_ a: PHAsset) -> AssetReply? {
        guard a.mediaType == .image || a.mediaType == .video else { return nil }
        let rs = PHAssetResource.assetResources(for: a).map(info)
        return AssetReply.of(id: a.localIdentifier, modified: a.modificationDate ?? a.creationDate, isVideo: a.mediaType == .video, rs)
    }

    static func info(_ r: PHAssetResource) -> ResourceInfo {
        // `fileSize` and `locallyAvailable` are not in the headers; both are read by key, and a
        // missing one is treated as unknown.
        ResourceInfo(
            kind: kind(r.type),
            filename: r.originalFilename,
            uti: r.uniformTypeIdentifier,
            size: (r.value(forKey: "fileSize") as? NSNumber)?.int64Value,
            local: (r.value(forKey: "locallyAvailable") as? NSNumber)?.boolValue
        )
    }

    static func kind(_ t: PHAssetResourceType) -> ResourceKind {
        switch t {
        case .photo: return .photo
        case .fullSizePhoto: return .fullSizePhoto
        case .video: return .video
        case .fullSizeVideo: return .fullSizeVideo
        case .pairedVideo: return .pairedVideo
        case .fullSizePairedVideo: return .fullSizePairedVideo
        default: return .other
        }
    }

    static func asset(_ id: String) throws -> PHAsset {
        try requireAccess()
        guard let a = PHAsset.fetchAssets(withLocalIdentifiers: [id], options: nil).firstObject else {
            throw PhotosError("\(id) is no longer in the Photos library")
        }
        return a
    }

    /// Write the asset's photo or video into `dir` (an empty directory of the caller's), and a Live
    /// Photo's video beside it under the same stem. With `network`, an original in iCloud only is
    /// downloaded (D3); without, reading it fails. PhotoKit streams to the file: memory stays small
    /// whatever the size.
    public static func read(id: String, into dir: URL, network: Bool) throws -> ReadReply {
        let a = try asset(id)
        let resources = PHAssetResource.assetResources(for: a)
        let rs = resources.map(info)
        guard let i = Choice.main(isVideo: a.mediaType == .video, rs) else {
            throw PhotosError("\(id) has no photo or video to read")
        }
        let name = Choice.name(rs, main: i)
        let file = dir.appendingPathComponent(name)
        try write(resources[i], to: file, network: network)
        var video: String?
        if a.mediaType == .image, let p = Choice.paired(rs) {
            let v = dir.appendingPathComponent(Choice.pairedName(still: name, rs[p]))
            // The still is in; a video that cannot be read leaves it a still.
            if (try? write(resources[p], to: v, network: network)) != nil { video = v.path }
        }
        return ReadReply(path: file.path, video: video)
    }

    static func write(_ r: PHAssetResource, to url: URL, network: Bool) throws {
        let o = PHAssetResourceRequestOptions()
        o.isNetworkAccessAllowed = network
        let done = DispatchSemaphore(value: 0)
        var failure: Error?
        PHAssetResourceManager.default().writeData(for: r, toFile: url, options: o) { e in
            failure = e
            done.signal()
        }
        done.wait()
        if let failure {
            try? FileManager.default.removeItem(at: url)
            throw PhotosError(network ? "could not read \(r.originalFilename): \(failure.localizedDescription)"
                                      : "\(r.originalFilename) is not on this device")
        }
    }

    /// A poster frame (JPEG) for the video `id`, a tenth of the way in; nil for a photo or a video
    /// that is not on this device.
    public static func frame(id: String) throws -> Data? {
        let a = try asset(id)
        guard a.mediaType == .video else { return nil }
        let o = PHVideoRequestOptions()
        o.isNetworkAccessAllowed = false
        o.version = .current
        let got = DispatchSemaphore(value: 0)
        var av: AVAsset?
        PHImageManager.default().requestAVAsset(forVideo: a, options: o) { asset, _, _ in
            av = asset
            got.signal()
        }
        got.wait()
        guard let av else { return nil }
        let g = AVAssetImageGenerator(asset: av)
        g.appliesPreferredTrackTransform = true
        g.maximumSize = CGSize(width: 1280, height: 1280)
        let at = CMTime(seconds: max(0, a.duration * 0.1), preferredTimescale: 600)
        let made = DispatchSemaphore(value: 0)
        var image: CGImage?
        g.generateCGImageAsynchronously(for: at) { img, _, _ in
            image = img
            made.signal()
        }
        made.wait()
        return image.flatMap(jpeg)
    }

    static func jpeg(_ image: CGImage) -> Data? {
        let out = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(out, UTType.jpeg.identifier as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(dest, image, [kCGImageDestinationLossyCompressionQuality: 0.85] as CFDictionary)
        return CGImageDestinationFinalize(dest) ? out as Data : nil
    }
}
