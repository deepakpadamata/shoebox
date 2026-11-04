import Foundation
import Testing
@testable import ShoeboxPhotos

let still = ResourceInfo(kind: .photo, filename: "IMG_0001.HEIC", uti: "public.heic", size: 2_000_000, local: true)
let render = ResourceInfo(kind: .fullSizePhoto, filename: "FullSizeRender.jpeg", uti: "public.jpeg", size: 1_500_000, local: true)
let paired = ResourceInfo(kind: .pairedVideo, filename: "IMG_0001.MOV", uti: "com.apple.quicktime-movie", size: 3_000_000)
let pairedRender = ResourceInfo(kind: .fullSizePairedVideo, filename: "FullSizeRender.mov", uti: "com.apple.quicktime-movie")
let adjustments = ResourceInfo(kind: .other, filename: "Adjustments.plist", uti: "com.apple.property-list")

@Test func anUneditedPhotoIsItsOriginal() {
    let rs = [still, paired]
    #expect(Choice.main(isVideo: false, rs) == 0)
    #expect(Choice.name(rs, main: 0) == "IMG_0001.HEIC")
}

@Test func anEditedPhotoIsItsRenderUnderTheOriginalName() {
    let rs = [still, adjustments, render]
    #expect(Choice.main(isVideo: false, rs) == 2)
    #expect(Choice.name(rs, main: 2) == "IMG_0001.jpeg")
}

@Test func aVideoIsNeverAPhoto() {
    let video = ResourceInfo(kind: .video, filename: "IMG_0002.MOV", uti: "com.apple.quicktime-movie")
    let edited = ResourceInfo(kind: .fullSizeVideo, filename: "FullSizeRender.mov", uti: "com.apple.quicktime-movie")
    #expect(Choice.main(isVideo: true, [video]) == 0)
    #expect(Choice.main(isVideo: true, [video, edited]) == 1)
    #expect(Choice.main(isVideo: false, [video]) == nil)
    #expect(Choice.name([video, edited], main: 1) == "IMG_0002.mov")
}

@Test func aLivePhotosVideoMatchesItsStillsVersion() {
    #expect(Choice.paired([still, paired]) == 1)
    #expect(Choice.paired([still, render, paired, pairedRender]) == 3)
    // Edited without the video being re-rendered (an older edit): the original video.
    #expect(Choice.paired([still, render, paired]) == 2)
    #expect(Choice.paired([still]) == nil)
    #expect(Choice.pairedName(still: "IMG_0001.jpeg", pairedRender) == "IMG_0001.mov")
}

@Test func namesNeverLeaveTheDirectory() {
    let odd = ResourceInfo(kind: .photo, filename: "../..hidden/x.JPG", uti: "public.jpeg")
    let name = Choice.name([odd], main: 0)
    #expect(!name.contains("/"))
    #expect(!name.hasPrefix("."))
    let blank = ResourceInfo(kind: .photo, filename: ".jpg", uti: "public.jpeg")
    #expect(Choice.name([blank], main: 0) == "jpg")
}

@Test func mimeTypesComeFromTheTypeOrTheExtension() {
    #expect(Mime.of(uti: "public.heic", filename: "a.HEIC") == "image/heic")
    #expect(Mime.of(uti: "public.jpeg", filename: "a") == "image/jpeg")
    #expect(Mime.of(uti: "com.apple.quicktime-movie", filename: "a.MOV") == "video/quicktime")
    #expect(Mime.of(uti: "not.a.type", filename: "a.mp4") == "video/mp4")
    #expect(Mime.of(uti: "not.a.type", filename: "a") == "application/octet-stream")
}

@Test func anAssetReplyHasTheAndroidShapeAndTheLibraryFlags() throws {
    let when = Date(timeIntervalSince1970: 1_700_000_000.25)
    let cloud = ResourceInfo(kind: .photo, filename: "IMG_0003.HEIC", uti: "public.heic", size: 10, local: false)
    let r = try #require(AssetReply.of(id: "A/L0/001", modified: when, isVideo: false, [cloud, paired]))
    #expect(r.modified_ms == 1_700_000_000_250)
    #expect(r.cloud_only && r.live && r.library)
    let json = try JSONSerialization.jsonObject(with: JSONEncoder().encode(r)) as! [String: Any]
    #expect(Set(json.keys) == ["id", "modified_ms", "size", "mime", "library", "cloud_only", "name", "live"])
    // Unknown availability counts as here; a video is never live.
    let unknown = ResourceInfo(kind: .video, filename: "v.mp4", uti: "public.mpeg-4")
    let v = try #require(AssetReply.of(id: "v", modified: nil, isVideo: true, [unknown, paired]))
    #expect(!v.cloud_only && !v.live && v.size == 0 && v.modified_ms == 0)
    #expect(AssetReply.of(id: "x", modified: nil, isVideo: false, [adjustments]) == nil)
}

@Test func tokensRoundTripAndForeignOnesMeanAFullScan() {
    let bytes = Data([0, 1, 2, 250])
    #expect(Token.decode(Token.encode(bytes)) == bytes)
    #expect(Token.decode(nil) == nil)
    #expect(Token.decode("") == nil)
    // The fake source's and Android's tokens are plain numbers.
    #expect(Token.decode("1234") == nil)
    #expect(Token.decode("pk1:not base64!") == nil)
}

@Test func laterChangesWin() {
    let set = ChangeSet.merge([
        (inserted: ["a", "b"], updated: [], deleted: []),
        (inserted: [], updated: ["a", "c"], deleted: ["b"]),
        (inserted: ["d"], updated: [], deleted: ["c", "e"]),
        (inserted: ["e"], updated: [], deleted: []),
    ])
    #expect(set.changed == ["a", "d", "e"])
    #expect(set.deleted == ["b", "c"])
}
