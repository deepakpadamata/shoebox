// iOS only (I2-01): the system photo picker and file picker, and the device's power and network
// state, behind C entry points like the rest of the bridge. The pickers are out-of-process
// system UI: they need no Photos permission, and they hand over copies of what was chosen, so
// each chosen file is copied into `dir` and the library keeps its own copy (PhotoKit has no
// paths to reference).
#if os(iOS)
import Foundation
import Network
import PhotosUI
import UIKit
import UniformTypeIdentifiers

private struct Chosen: Encodable {
    let name: String
    let path: String
}

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

private struct PickerError: Error, CustomStringConvertible {
    let description: String
}

/// The view controller that is on screen now, to present the picker over.
@MainActor private func topController() -> UIViewController? {
    let scene = UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }
        .first { $0.activationState == .foregroundActive } ?? UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
    var top = scene?.windows.first { $0.isKeyWindow }?.rootViewController ?? scene?.windows.first?.rootViewController
    while let next = top?.presentedViewController { top = next }
    return top
}

/// Holds the picker's delegate while it is on screen (a view controller keeps its delegate weakly).
@MainActor private final class Session: NSObject, PHPickerViewControllerDelegate, UIDocumentPickerDelegate {
    static var current: Session?
    var finish: (([URL]?, [PHPickerResult]?) -> Void)?

    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        finish?(nil, results)
        Session.current = nil
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        finish?(urls, nil)
        Session.current = nil
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        finish?([], nil)
        Session.current = nil
    }
}

/// Copy a picked file into `dir` under a name that is not taken.
private func place(_ src: URL, name: String, in dir: URL) throws -> Chosen {
    var target = dir.appendingPathComponent(name)
    var n = 1
    while FileManager.default.fileExists(atPath: target.path) {
        let stem = (name as NSString).deletingPathExtension
        let ext = (name as NSString).pathExtension
        target = dir.appendingPathComponent(ext.isEmpty ? "\(stem) \(n)" : "\(stem) \(n).\(ext)")
        n += 1
    }
    try FileManager.default.copyItem(at: src, to: target)
    return Chosen(name: target.lastPathComponent, path: target.path)
}

/// The photo picker (`files` false) or the document picker (`files` true); copies of the choices.
@_cdecl("sbp_pick")
public func sbp_pick(_ files: Bool, _ dir: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar> {
    let dir = URL(fileURLWithPath: String(cString: dir), isDirectory: true)
    return reply {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let done = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var urls: [URL]?
        nonisolated(unsafe) var results: [PHPickerResult]?
        nonisolated(unsafe) var problem: String?
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let top = topController() else {
                    problem = "there is no screen to show the picker on"
                    done.signal()
                    return
                }
                let session = Session()
                session.finish = { u, r in
                    urls = u
                    results = r
                    done.signal()
                }
                Session.current = session
                if files {
                    let doc = UIDocumentPickerViewController(forOpeningContentTypes: [.image, .movie], asCopy: true)
                    doc.allowsMultipleSelection = true
                    doc.delegate = session
                    top.present(doc, animated: true)
                } else {
                    var config = PHPickerConfiguration()
                    config.selectionLimit = 0
                    config.filter = .any(of: [.images, .videos])
                    // The original, not a transcoded copy.
                    config.preferredAssetRepresentationMode = .current
                    let picker = PHPickerViewController(configuration: config)
                    picker.delegate = session
                    top.present(picker, animated: true)
                }
            }
        }
        done.wait()
        if let problem { throw PickerError(description: problem) }
        if let urls {
            return try urls.map { try place($0, name: $0.lastPathComponent, in: dir) }
        }
        var chosen: [Chosen] = []
        for r in results ?? [] {
            let provider = r.itemProvider
            // The first format the item is stored in that is an image or a video (HEIC stays HEIC).
            guard let type = provider.registeredTypeIdentifiers
                .compactMap({ UTType($0) })
                .first(where: { $0.conforms(to: .image) || $0.conforms(to: .movie) }) else { continue }
            let loaded = DispatchSemaphore(value: 0)
            nonisolated(unsafe) var made: Chosen?
            nonisolated(unsafe) var failure: Error?
            provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
                defer { loaded.signal() }
                guard let url else { failure = error; return }
                // The file goes when this closure returns: copy it now.
                do { made = try place(url, name: url.lastPathComponent, in: dir) } catch { failure = error }
            }
            loaded.wait()
            if let made { chosen.append(made) } else if let failure { throw failure }
        }
        return chosen
    }
}

/// With only some photos shared: iOS's own picker to share more or fewer. Returns when it is
/// dismissed. (Asking for access again does nothing once the person has chosen "limited".)
@_cdecl("sbp_manage_limited")
public func sbp_manage_limited() -> UnsafeMutablePointer<CChar> {
    reply {
        let done = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var problem: String?
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let top = topController() else {
                    problem = "there is no screen to show the picker on"
                    done.signal()
                    return
                }
                PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: top) { _ in done.signal() }
            }
        }
        done.wait()
        if let problem { throw PickerError(description: problem) }
        return true
    }
}

private struct Conditions: Encodable {
    let on_wifi: Bool
    let charging: Bool
    let battery_pct: Int
}

/// Power and network, for the sync rules: Wi-Fi (or wired) and charging.
@_cdecl("sbp_conditions")
public func sbp_conditions() -> UnsafeMutablePointer<CChar> {
    reply {
        nonisolated(unsafe) var charging = false
        nonisolated(unsafe) var pct = 100
        DispatchQueue.main.sync {
            UIDevice.current.isBatteryMonitoringEnabled = true
            let s = UIDevice.current.batteryState
            charging = s == .charging || s == .full
            let level = UIDevice.current.batteryLevel
            pct = level < 0 ? 100 : Int((level * 100).rounded())
        }
        let monitor = NWPathMonitor()
        let got = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var wifi = false
        monitor.pathUpdateHandler = { path in
            wifi = path.status == .satisfied && !path.isExpensive
                && (path.usesInterfaceType(.wifi) || path.usesInterfaceType(.wiredEthernet))
            got.signal()
        }
        monitor.start(queue: DispatchQueue(label: "shoebox.conditions"))
        _ = got.wait(timeout: .now() + 2)
        monitor.cancel()
        return Conditions(on_wifi: wifi, charging: charging, battery_pct: pct)
    }
}
#endif
