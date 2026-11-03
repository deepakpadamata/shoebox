// PhotoKit probe for the P1-01 spike (planning/issues/P1-01-spike-photokit-on-this-mac.md).
//
// Reads the system Photos library and prints numbers; it never changes the library. Run it through
// `run.sh`, which builds a signed app bundle so macOS asks for Photos access on its own behalf.
//
//   auth                  ask for access, print the state
//   enumerate             every asset: fetch time, counts by kind, and the cost of listing resources
//   resources [n]         resource types seen on n assets (default 2000), edits, Live Photos
//   local [n]             which originals are on this Mac, by two methods, on n assets (default 300)
//   stream [n]            stream the largest local video and n photos (default 20) through SHA-256,
//                         export the same resources to files and compare; throughput and peak memory
//   token <file>          save the current change token
//   changes <file>        what changed since the token in <file>
import CryptoKit
import Foundation
import Photos

setvbuf(stdout, nil, _IOLBF, 0)

func now() -> Double { Date().timeIntervalSince1970 }
func ms(_ t0: Double) -> String { String(format: "%.0f ms", (now() - t0) * 1000) }
func peakRSSMB() -> Double {
    var u = rusage()
    getrusage(RUSAGE_SELF, &u)
    return Double(u.ru_maxrss) / 1_048_576  // bytes on macOS
}

func authState(_ s: PHAuthorizationStatus) -> String {
    switch s {
    case .authorized: return "granted"
    case .limited: return "limited"
    case .denied: return "denied"
    case .restricted: return "restricted"
    case .notDetermined: return "not determined"
    @unknown default: return "unknown"
    }
}

func resourceName(_ t: PHAssetResourceType) -> String {
    switch t {
    case .photo: return "photo"
    case .video: return "video"
    case .audio: return "audio"
    case .alternatePhoto: return "alternatePhoto"
    case .fullSizePhoto: return "fullSizePhoto"
    case .fullSizeVideo: return "fullSizeVideo"
    case .adjustmentData: return "adjustmentData"
    case .adjustmentBasePhoto: return "adjustmentBasePhoto"
    case .pairedVideo: return "pairedVideo"
    case .fullSizePairedVideo: return "fullSizePairedVideo"
    case .adjustmentBasePairedVideo: return "adjustmentBasePairedVideo"
    case .adjustmentBaseVideo: return "adjustmentBaseVideo"
    case .photoProxy: return "photoProxy"
    @unknown default: return "other(\(t.rawValue))"
    }
}

func requireAccess() {
    let s = PHPhotoLibrary.authorizationStatus(for: .readWrite)
    guard s == .authorized || s == .limited else {
        print("no access (\(authState(s))): run `auth` first")
        exit(2)
    }
}

func allAssets() -> PHFetchResult<PHAsset> {
    PHAsset.fetchAssets(with: nil)
}

/// `n` assets spread evenly over the whole library.
func sample(_ all: PHFetchResult<PHAsset>, _ n: Int) -> [PHAsset] {
    let count = all.count
    guard count > 0 else { return [] }
    let step = max(1, count / max(1, n))
    return stride(from: 0, to: count, by: step).prefix(n).map { all.object(at: $0) }
}

/// The resource a backup would take as the photo (D2: the current version).
func mainResource(_ a: PHAsset) -> PHAssetResource? {
    let rs = PHAssetResource.assetResources(for: a)
    let order: [PHAssetResourceType] =
        a.mediaType == .video ? [.fullSizeVideo, .video] : [.fullSizePhoto, .photo]
    for t in order { if let r = rs.first(where: { $0.type == t }) { return r } }
    return nil
}

func size(_ r: PHAssetResource) -> Int64 { (r.value(forKey: "fileSize") as? Int64) ?? -1 }

/// Stream `r` without network; returns (sha256 hex, bytes, error).
func streamHash(_ r: PHAssetResource, network: Bool = false, stopAfterFirstChunk: Bool = false)
    -> (String?, Int64, Error?)
{
    let o = PHAssetResourceRequestOptions()
    o.isNetworkAccessAllowed = network
    var h = SHA256()
    var n: Int64 = 0
    var err: Error?
    let done = DispatchSemaphore(value: 0)
    let mgr = PHAssetResourceManager.default()
    var id: PHAssetResourceDataRequestID = 0
    var stopped = false
    id = mgr.requestData(for: r, options: o, dataReceivedHandler: { d in
        h.update(data: d)
        n += Int64(d.count)
        if stopAfterFirstChunk && !stopped {
            stopped = true
            mgr.cancelDataRequest(id)
        }
    }, completionHandler: { e in
        err = e
        done.signal()
    })
    done.wait()
    if stopped { return (nil, n, nil) }
    if err != nil { return (nil, n, err) }
    return (h.finalize().map { String(format: "%02x", $0) }.joined(), n, nil)
}

func fileHash(_ url: URL) -> String? {
    guard let fh = try? FileHandle(forReadingFrom: url) else { return nil }
    var h = SHA256()
    while let d = try? fh.read(upToCount: 4 << 20), !d.isEmpty { h.update(data: d) }
    return h.finalize().map { String(format: "%02x", $0) }.joined()
}

let args = Array(CommandLine.arguments.dropFirst())
let cmd = args.first ?? "help"
let arg1 = args.count > 1 ? args[1] : nil

switch cmd {
case "auth":
    print("before: \(authState(PHPhotoLibrary.authorizationStatus(for: .readWrite)))")
    let done = DispatchSemaphore(value: 0)
    PHPhotoLibrary.requestAuthorization(for: .readWrite) { s in
        print("after: \(authState(s))")
        done.signal()
    }
    done.wait()

case "enumerate":
    requireAccess()
    var t0 = now()
    let all = allAssets()
    let n = all.count
    print("fetch: \(n) assets in \(ms(t0))")
    print("unfiltered fetch: \(PHAsset.fetchAssets(with: nil).count); images \(PHAsset.fetchAssets(with: .image, options: nil).count); albums \(PHAssetCollection.fetchAssetCollections(with: .album, subtype: .any, options: nil).count)")
    t0 = now()
    var images = 0, videos = 0, live = 0, favs = 0, cloudShared = 0
    var ids = [String]()
    ids.reserveCapacity(n)
    all.enumerateObjects { a, _, _ in
        ids.append(a.localIdentifier)
        _ = a.modificationDate
        if a.mediaType == .image { images += 1 }
        if a.mediaType == .video { videos += 1 }
        if a.mediaSubtypes.contains(.photoLive) { live += 1 }
        if a.isFavorite { favs += 1 }
        if a.sourceType == .typeCloudShared { cloudShared += 1 }
    }
    print("walk (id, modified, kind): \(ms(t0)); images \(images), videos \(videos), live \(live), favourites \(favs), shared-album \(cloudShared)")
    print("example id: \(ids.first ?? "-")")
    let k = min(n, 1000)
    t0 = now()
    for a in sample(all, k) { _ = PHAssetResource.assetResources(for: a) }
    let per = (now() - t0) / Double(max(k, 1))
    print(String(format: "resources listing: %.2f ms per asset over %d; for all %d ≈ %.1f s", per * 1000, k, n, per * Double(n)))
    print(String(format: "peak memory %.0f MB", peakRSSMB()))

case "resources":
    requireAccess()
    let k = Int(arg1 ?? "") ?? 2000
    let all = allAssets()
    var hist = [String: Int]()
    var combos = [String: Int]()
    var edited = 0, liveWithPaired = 0, liveTotal = 0
    for a in sample(all, k) {
        let rs = PHAssetResource.assetResources(for: a)
        let names = rs.map { resourceName($0.type) }
        for x in Set(names) { hist[x, default: 0] += 1 }
        combos[(a.mediaType == .video ? "video: " : "image: ") + names.sorted().joined(separator: "+"), default: 0] += 1
        if rs.contains(where: { $0.type == .adjustmentData }) { edited += 1 }
        if a.mediaSubtypes.contains(.photoLive) {
            liveTotal += 1
            if rs.contains(where: { $0.type == .pairedVideo }) { liveWithPaired += 1 }
        }
    }
    print("sampled \(min(k, all.count)) of \(all.count)")
    print("assets having each resource type:")
    for (k, v) in hist.sorted(by: { $0.value > $1.value }) { print("  \(k): \(v)") }
    print("combinations:")
    for (k, v) in combos.sorted(by: { $0.value > $1.value }) { print("  \(v)\t\(k)") }
    print("edited (adjustmentData present): \(edited); live: \(liveTotal), with pairedVideo: \(liveWithPaired)")

case "local":
    requireAccess()
    let k = Int(arg1 ?? "") ?? 300
    let all = allAssets()
    var kvcLocal = 0, kvcRemote = 0, kvcUnknown = 0, probeLocal = 0, probeRemote = 0, disagree = 0
    var errors = [String: Int]()
    let t0 = now()
    for a in sample(all, k) {
        guard let r = mainResource(a) else { continue }
        let kvc = r.value(forKey: "locallyAvailable") as? Bool
        switch kvc {
        case true?: kvcLocal += 1
        case false?: kvcRemote += 1
        case nil: kvcUnknown += 1
        }
        let (_, got, err) = streamHash(r, network: false, stopAfterFirstChunk: true)
        let local = err == nil && got > 0
        if local { probeLocal += 1 } else {
            probeRemote += 1
            if let e = err as NSError? { errors["\(e.domain) \(e.code)", default: 0] += 1 }
        }
        if let kvc, kvc != local { disagree += 1 }
    }
    print("sampled \(min(k, all.count)) in \(ms(t0))")
    print("KVC locallyAvailable: local \(kvcLocal), not local \(kvcRemote), no value \(kvcUnknown)")
    print("first-chunk probe without network: local \(probeLocal), not local \(probeRemote)")
    print("errors when not local: \(errors)")
    print("the two methods disagree on \(disagree)")

case "stream":
    requireAccess()
    let k = Int(arg1 ?? "") ?? 20
    let all = allAssets()
    let out = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("photokit-probe")
    try? FileManager.default.removeItem(at: out)
    try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
    // The largest local video among a sample, then photos.
    var picks = [PHAssetResource]()
    let vo = PHFetchOptions()
    vo.sortDescriptors = [NSSortDescriptor(key: "duration", ascending: false)]
    let vids = PHAsset.fetchAssets(with: .video, options: vo)
    for i in 0..<min(vids.count, 50) {
        if let r = mainResource(vids.object(at: i)), (r.value(forKey: "locallyAvailable") as? Bool) == true {
            picks.append(r)
            break
        }
    }
    for a in sample(all, k * 4) where a.mediaType == .image {
        if picks.count > k { break }
        if let r = mainResource(a), (r.value(forKey: "locallyAvailable") as? Bool) == true { picks.append(r) }
    }
    var bytes: Int64 = 0, secs = 0.0, matched = 0, mismatched = 0, failed = 0
    let rss0 = peakRSSMB()
    for (i, r) in picks.enumerated() {
        let t0 = now()
        let (hash, n, err) = streamHash(r)
        let dt = now() - t0
        if let err { print("  stream failed: \(err)"); failed += 1; continue }
        bytes += n
        secs += dt
        let file = out.appendingPathComponent("\(i)")
        let done = DispatchSemaphore(value: 0)
        var werr: Error?
        PHAssetResourceManager.default().writeData(for: r, toFile: file, options: nil) { e in
            werr = e
            done.signal()
        }
        done.wait()
        if let werr { print("  export failed: \(werr)"); failed += 1; continue }
        let fh = fileHash(file)
        if fh == hash { matched += 1 } else { mismatched += 1 }
        if i == 0 {
            print(String(format: "  largest video: %.1f MB streamed in %.2f s (%.0f MB/s), peak memory %.0f MB (was %.0f)",
                         Double(n) / 1e6, dt, Double(n) / 1e6 / dt, peakRSSMB(), rss0))
        }
        try? FileManager.default.removeItem(at: file)
    }
    print(String(format: "streamed %d resources, %.1f MB in %.2f s (%.0f MB/s)", picks.count - failed, Double(bytes) / 1e6, secs, Double(bytes) / 1e6 / max(secs, 0.001)))
    print("stream hash = exported file hash: \(matched), differ: \(mismatched), failed: \(failed)")
    print(String(format: "peak memory %.0f MB", peakRSSMB()))
    try? FileManager.default.removeItem(at: out)

case "token":
    requireAccess()
    guard let path = arg1 else { print("token <file>"); exit(1) }
    let tok = PHPhotoLibrary.shared().currentChangeToken
    let data = try NSKeyedArchiver.archivedData(withRootObject: tok, requiringSecureCoding: true)
    try data.write(to: URL(fileURLWithPath: path))
    print("saved a \(data.count)-byte token to \(path)")

case "changes":
    requireAccess()
    guard let path = arg1 else { print("changes <file>"); exit(1) }
    let data = try Data(contentsOf: URL(fileURLWithPath: path))
    guard let tok = try NSKeyedUnarchiver.unarchivedObject(ofClass: PHPersistentChangeToken.self, from: data) else {
        print("not a token"); exit(1)
    }
    let t0 = now()
    let changes = try PHPhotoLibrary.shared().fetchPersistentChanges(since: tok)
    var batches = 0
    for c in changes {
        batches += 1
        let d = try c.changeDetails(for: .asset)
        print("batch \(batches): inserted \(d.insertedLocalIdentifiers.count), updated \(d.updatedLocalIdentifiers.count), deleted \(d.deletedLocalIdentifiers.count)")
        for id in d.insertedLocalIdentifiers { print("  + \(id)") }
        for id in d.updatedLocalIdentifiers {
            let a = PHAsset.fetchAssets(withLocalIdentifiers: [id], options: nil).firstObject
            let rs = a.map { PHAssetResource.assetResources(for: $0).map { resourceName($0.type) } } ?? []
            print("  ~ \(id) favourite=\(a?.isFavorite ?? false) modified=\(a?.modificationDate.map { "\($0)" } ?? "-") resources=\(rs)")
        }
        for id in d.deletedLocalIdentifiers { print("  - \(id)") }
    }
    print("\(batches) change batches in \(ms(t0))")

default:
    print("usage: auth | enumerate | resources [n] | local [n] | stream [n] | token <file> | changes <file>")
}
