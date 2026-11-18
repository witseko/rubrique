// Drive a window for a staged screenshot: find it, move the pointer, click
// and type in it. Pointer positions are in points from the window's top
// left corner and are refused outside the window, so a click never lands
// somewhere else on the screen. Posting events needs Accessibility access
// for the terminal that runs this.
//
//   swift drive.swift window PID TITLE-PART   prints "id x y width height"
//   swift drive.swift activate PID              brings the app to the front
//   swift drive.swift move ID X Y               moves the pointer
//   swift drive.swift click ID X Y              clicks there
//   swift drive.swift type TEXT                 types the text
//   swift drive.swift key NAME                  presses enter, escape, tab or cmd+LETTER
import AppKit
import CoreGraphics
import Foundation

struct Window {
    let id: CGWindowID
    let frame: CGRect
}

/// On-screen windows, front to back.
func windows() -> [(pid: Int, title: String, window: Window)] {
    let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as! [[String: Any]]
    return list.compactMap { info in
        guard let id = info[kCGWindowNumber as String] as? Int,
              let bounds = info[kCGWindowBounds as String] as? [String: Any],
              let frame = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return nil }
        guard let pid = info[kCGWindowOwnerPID as String] as? Int else { return nil }
        let title = info[kCGWindowName as String] as? String ?? ""
        return (pid, title, Window(id: CGWindowID(id), frame: frame))
    }
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

/// The window `id`, which must still be on screen.
func window(_ id: String) -> Window {
    guard let number = Int(id), let found = windows().first(where: { $0.window.id == CGWindowID(number) }) else {
        fail("window \(id) is not on screen")
    }
    return found.window
}

/// The screen point for (x, y) inside `window`; refused outside it.
func point(in window: Window, _ xText: String, _ yText: String) -> CGPoint {
    guard let x = Double(xText), let y = Double(yText) else { fail("bad point \(xText) \(yText)") }
    guard x >= 0, y >= 0, CGFloat(x) <= window.frame.width, CGFloat(y) <= window.frame.height else {
        fail("point \(x) \(y) is outside the window (\(Int(window.frame.width)) x \(Int(window.frame.height)))")
    }
    return CGPoint(x: window.frame.minX + CGFloat(x), y: window.frame.minY + CGFloat(y))
}

func post(_ event: CGEvent?) {
    guard let event else { fail("cannot make the event") }
    event.post(tap: .cghidEventTap)
    usleep(60_000)
}

func move(to point: CGPoint) {
    post(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left))
}

func click(at point: CGPoint) {
    move(to: point)
    usleep(150_000)
    post(CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left))
    post(CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left))
}

/// Press and release a key, with modifier flags.
func press(_ code: CGKeyCode, _ flags: CGEventFlags = []) {
    let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true)
    down?.flags = flags
    post(down)
    let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false)
    up?.flags = flags
    post(up)
}

/// The left Command key.
let commandKey: CGKeyCode = 55

/// Cmd+key: the Command key goes down and up around it, as on a real
/// keyboard. Setting the flag on the letter alone is not enough: the app
/// learns about modifiers from the modifier key's own events, and would
/// take every later letter as a Command shortcut.
func pressWithCommand(_ code: CGKeyCode) {
    let commandDown = CGEvent(keyboardEventSource: nil, virtualKey: commandKey, keyDown: true)
    commandDown?.flags = .maskCommand
    post(commandDown)
    press(code, .maskCommand)
    let commandUp = CGEvent(keyboardEventSource: nil, virtualKey: commandKey, keyDown: false)
    commandUp?.flags = []
    post(commandUp)
}

/// Type `text` one character at a time (as unicode, no key codes needed).
func type(_ text: String) {
    for character in text {
        var units = Array(String(character).utf16)
        let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true)
        down?.flags = []
        down?.keyboardSetUnicodeString(stringLength: units.count, unicodeString: &units)
        post(down)
        let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false)
        up?.flags = []
        up?.keyboardSetUnicodeString(stringLength: units.count, unicodeString: &units)
        post(up)
    }
}

/// ANSI key codes for the letters, for cmd+LETTER.
let letterCodes: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12, "w": 13,
    "e": 14, "r": 15, "y": 16, "t": 17, "o": 31, "u": 32, "i": 34, "p": 35, "l": 37, "j": 38, "k": 40, "n": 45, "m": 46,
]

let args = CommandLine.arguments.dropFirst()
switch args.first ?? "" {
case "permissions" where args.count == 2:
    guard CGPreflightScreenCaptureAccess() else { fail("Allow Screen Recording for this terminal, then run again.") }
    if args.last == "input", !AXIsProcessTrusted() {
        fail("Allow Accessibility for this terminal, then run again.")
    }
case "window" where args.count == 3:
    guard let pid = Int(args[args.startIndex + 1]) else { fail("invalid process ID") }
    let part = args[args.startIndex + 2]
    if let found = windows().first(where: { $0.pid == pid && $0.title.contains(part) }) {
        let f = found.window.frame
        print("\(found.window.id) \(Int(f.minX)) \(Int(f.minY)) \(Int(f.width)) \(Int(f.height))")
    }
case "activate" where args.count == 2:
    guard let pid = Int32(args[args.startIndex + 1]), let app = NSRunningApplication(processIdentifier: pid) else {
        fail("no process \(args[args.startIndex + 1])")
    }
    app.activate()
case "move" where args.count == 4:
    move(to: point(in: window(args[args.startIndex + 1]), args[args.startIndex + 2], args[args.startIndex + 3]))
case "click" where args.count == 4:
    click(at: point(in: window(args[args.startIndex + 1]), args[args.startIndex + 2], args[args.startIndex + 3]))
case "type" where args.count == 2:
    type(args[args.startIndex + 1])
case "key" where args.count == 2:
    let name = args[args.startIndex + 1]
    switch name {
    case "enter": press(36)
    case "escape": press(53)
    case "tab": press(48)
    default:
        guard name.hasPrefix("cmd+"), let letter = name.dropFirst(4).first, let code = letterCodes[letter] else {
            fail("unknown key \(name)")
        }
        pressWithCommand(code)
    }
default:
    fail("usage: drive.swift window|activate|move|click|type|key ... (see the top of the file)")
}
