// pshot <executable> <captureMs> — the window probe for launch.mjs.
// Launches the executable (environment inherited, stdio to /dev/null) and times, from just before
// launch: the first on-screen window owned by its PID (CGWindowList, ~4 ms polling), then captures
// that window continuously with ScreenCaptureKit for <captureMs> to find the first non-blank frame
// and the frame after which the window no longer changes (visually complete). Prints one JSON line
// and exits, leaving the app running.
import AppKit
import CoreGraphics
import Foundation
import ScreenCaptureKit

func now() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }

func mainWindow(_ pid: pid_t) -> CGWindowID? {
  guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]]
  else { return nil }
  for w in list {
    guard (w[kCGWindowOwnerPID as String] as? Int32) == pid,
      (w[kCGWindowLayer as String] as? Int) == 0,
      ((w[kCGWindowAlpha as String] as? Double) ?? 1) > 0,
      let b = w[kCGWindowBounds as String] as? [String: Double], (b["Width"] ?? 0) > 300,
      let id = w[kCGWindowNumber as String] as? UInt32
    else { continue }
    return id
  }
  return nil
}

// A 96x60 grayscale thumbnail of the window.
let W = 96
let H = 60
func thumb(_ image: CGImage) -> [UInt8] {
  var px = [UInt8](repeating: 0, count: W * H)
  px.withUnsafeMutableBytes { buf in
    let ctx = CGContext(
      data: buf.baseAddress, width: W, height: H, bitsPerComponent: 8, bytesPerRow: W,
      space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
    ctx.interpolationQuality = .medium
    ctx.draw(image, in: CGRect(x: 0, y: 0, width: W, height: H))
  }
  return px
}

// Share of pixels more than 12 levels from the frame's median: a blank background is ~0.
func detail(_ f: [UInt8]) -> Double {
  let median = f.sorted()[f.count / 2]
  return Double(f.filter { abs(Int($0) - Int(median)) > 12 }.count) / Double(f.count)
}

func differs(_ a: [UInt8], _ b: [UInt8]) -> Double {
  var n = 0
  for i in 0..<a.count where abs(Int(a[i]) - Int(b[i])) > 12 { n += 1 }
  return Double(n) / Double(a.count)
}

func j(_ v: Double?) -> String { v.map { String(format: "%.1f", $0) } ?? "null" }

@main
struct Probe {
  static func main() async {
    let args = CommandLine.arguments
    guard args.count == 3, let captureMs = Double(args[2]) else {
      FileHandle.standardError.write("usage: pshot <exe> <captureMs>\n".data(using: .utf8)!)
      exit(2)
    }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: args[1])
    process.standardInput = FileHandle.nullDevice
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    let start = now()
    do { try process.run() } catch {
      print("{\"error\":\"launch failed\"}")
      exit(1)
    }
    let pid = process.processIdentifier

    var windowMs: Double? = nil
    var windowID: CGWindowID? = nil
    while now() - start < 30000, process.isRunning {
      if let id = mainWindow(pid) {
        windowMs = now() - start
        windowID = id
        break
      }
      usleep(4000)
    }

    var frames: [(t: Double, f: [UInt8])] = []
    var captureError: String? = nil
    if let windowID {
      do {
        var scWindow: SCWindow? = nil
        while scWindow == nil, now() - start < 30000 {
          let content = try await SCShareableContent.excludingDesktopWindows(
            false, onScreenWindowsOnly: true)
          scWindow = content.windows.first { $0.windowID == windowID }
        }
        if let scWindow {
          let filter = SCContentFilter(desktopIndependentWindow: scWindow)
          let config = SCStreamConfiguration()
          config.width = 384
          config.height = 240
          config.showsCursor = false
          let until = now() + captureMs
          while now() < until {
            let image = try await SCScreenshotManager.captureImage(
              contentFilter: filter, configuration: config)
            frames.append((now() - start, thumb(image)))
          }
        }
      } catch {
        captureError = "\(error)"
      }
    }

    var firstPaintMs: Double? = nil
    var completeMs: Double? = nil
    if let last = frames.last {
      firstPaintMs = frames.first { detail($0.f) > 0.03 }?.t
      // Earliest frame from which every later frame matches the final frame (under 0.5% changed).
      var i = frames.count - 1
      while i > 0, differs(frames[i - 1].f, last.f) < 0.005 { i -= 1 }
      completeMs = frames[i].t
    }
    let interval =
      frames.count > 1 ? (frames.last!.t - frames.first!.t) / Double(frames.count - 1) : 0
    let err =
      captureError.map { ",\"captureError\":\"\($0.replacingOccurrences(of: "\"", with: "'"))\"" }
      ?? ""
    print(
      "{\"pid\":\(pid),\"windowMs\":\(j(windowMs)),\"firstPaintMs\":\(j(firstPaintMs)),\"completeMs\":\(j(completeMs)),\"frames\":\(frames.count),\"frameIntervalMs\":\(j(interval)),\"firstFrameMs\":\(j(frames.first?.t)),\"firstFrameDetail\":\(j(frames.first.map { detail($0.f) * 100 })),\"running\":\(process.isRunning)\(err)}"
    )
    exit(0)
  }
}
