// C entry points over ShoeboxPhotos for the Rust app on macOS (P1-04, `macos_photos.rs`). Each
// takes C strings and returns a JSON C string the caller frees with `sbp_free`:
// `{"ok": <reply>}` or `{"error": "<what went wrong>"}`. All block: call from a background thread.
import Foundation
import ShoeboxPhotos

private struct Ok<V: Encodable>: Encodable { let ok: V }
private struct Failed: Encodable { let error: String }

private func reply<T: Encodable>(_ body: () throws -> T) -> UnsafeMutablePointer<CChar> {
    let data: Data
    do {
        data = try JSONEncoder().encode(Ok(ok: try body()))
    } catch {
        data = (try? JSONEncoder().encode(Failed(error: "\(error)"))) ?? Data(#"{"error":"?"}"#.utf8)
    }
    return strdup(String(decoding: data, as: UTF8.self))!
}

private func string(_ p: UnsafePointer<CChar>?) -> String? { p.map { String(cString: $0) } }

@_cdecl("sbp_permission")
public func sbp_permission() -> UnsafeMutablePointer<CChar> {
    reply { PhotoLibrary.permission() }
}

@_cdecl("sbp_request_permission")
public func sbp_request_permission() -> UnsafeMutablePointer<CChar> {
    reply { PhotoLibrary.requestPermission() }
}

/// `token`: null or empty for everything.
@_cdecl("sbp_scan")
public func sbp_scan(_ token: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar> {
    reply { try PhotoLibrary.scan(token: string(token).flatMap { $0.isEmpty ? nil : $0 }) }
}

@_cdecl("sbp_read")
public func sbp_read(_ id: UnsafePointer<CChar>, _ dir: UnsafePointer<CChar>, _ network: Bool) -> UnsafeMutablePointer<CChar> {
    reply {
        try PhotoLibrary.read(id: String(cString: id), into: URL(fileURLWithPath: String(cString: dir)), network: network)
    }
}

/// `{"ok": "<base64 JPEG>"}`, empty when there is no frame.
@_cdecl("sbp_frame")
public func sbp_frame(_ id: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar> {
    reply { try PhotoLibrary.frame(id: String(cString: id))?.base64EncodedString() ?? "" }
}

@_cdecl("sbp_free")
public func sbp_free(_ p: UnsafeMutablePointer<CChar>?) {
    free(p)
}
