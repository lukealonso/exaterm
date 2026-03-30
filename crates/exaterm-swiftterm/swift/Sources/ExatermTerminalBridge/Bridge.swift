import AppKit
import Foundation

@_cdecl("exaterm_terminal_bridge_force_link")
public func exaterm_terminal_bridge_force_link() {
}

/// ObjC-compatible bridge class that wraps a terminal view.
@objc(ExatermTerminalBridge)
public class ExatermTerminalBridge: NSObject {
    private let hostView: BridgeTerminalView
    @objc public var terminalView: NSView { hostView }

    private var inputHandler: ((Data) -> Void)?
    private var sizeHandler: ((Int, Int) -> Void)?

    @objc public override init() {
        self.hostView = BridgeTerminalView(frame: NSRect(x: 0, y: 0, width: 640, height: 480))
        super.init()
        self.hostView.bridge = self
    }

    @objc public init(frame: NSRect) {
        self.hostView = BridgeTerminalView(frame: frame)
        super.init()
        self.hostView.bridge = self
    }

    /// Feed raw PTY output bytes into the terminal emulator.
    @objc public func feed(_ data: Data) {
        let bytes = Array(data)
        hostView.feed(byteArray: bytes[...])
    }

    /// Reset terminal state before reusing the view for a different session.
    @objc public func clear() {
        hostView.clear()
    }

    /// Register a callback that fires when the terminal produces output (user keystrokes).
    @objc public func setInputHandler(_ block: @escaping (Data) -> Void) {
        self.inputHandler = block
    }

    /// Register a callback that fires when the terminal grid size changes.
    @objc public func setSizeHandler(_ block: @escaping (Int, Int) -> Void) {
        self.sizeHandler = block
    }

    /// Return the current terminal size as NSSize (width=cols, height=rows).
    @objc public func terminalSize() -> NSSize {
        let dims = hostView.getTerminal().getDims()
        return NSSize(width: CGFloat(dims.cols), height: CGFloat(dims.rows))
    }

    /// Set the terminal font.
    @objc public func setFontName(_ name: NSString, size: CGFloat) {
        hostView.setTerminalFont(name: name as String, size: size)
    }

    /// Set terminal foreground, background, and cursor colors.
    @objc public func setForegroundColor(_ fg: NSColor, backgroundColor bg: NSColor, cursorColor cursor: NSColor) {
        hostView.nativeForegroundColor = fg
        hostView.nativeBackgroundColor = bg
        hostView.caretColor = cursor
        hostView.layer?.backgroundColor = bg.cgColor
    }

    fileprivate func forwardInput(_ data: ArraySlice<UInt8>) {
        inputHandler?(Data(data))
    }

    fileprivate func forwardResize(cols: Int, rows: Int) {
        sizeHandler?(cols, rows)
    }
}

final class BridgeTerminalView: TerminalView, TerminalViewDelegate {
    weak var bridge: ExatermTerminalBridge?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        commonInit()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        commonInit()
    }

    private func commonInit() {
        terminalDelegate = self
        wantsLayer = true
        nativeBackgroundColor = .black
        nativeForegroundColor = NSColor(
            calibratedRed: CGFloat(0xcc) / 255.0,
            green: CGFloat(0xcc) / 255.0,
            blue: CGFloat(0xcc) / 255.0,
            alpha: 1.0
        )
        caretColor = .systemGreen
        layer?.backgroundColor = nativeBackgroundColor.cgColor

        do {
            try setUseMetal(false)
        } catch {
            // Stay on the CPU renderer if Metal cannot be configured.
        }
    }

    func setTerminalFont(name: String, size: CGFloat) {
        if let font = NSFont(name: name, size: size) {
            self.font = font
        } else {
            self.font = NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
        }
    }

    func clear() {
        getTerminal().resetToInitialState()
        needsDisplay = true
    }

    func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        bridge?.forwardResize(cols: newCols, rows: newRows)
    }

    func setTerminalTitle(source: TerminalView, title: String) {
    }

    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
    }

    func send(source: TerminalView, data: ArraySlice<UInt8>) {
        bridge?.forwardInput(data)
    }

    func scrolled(source: TerminalView, position: Double) {
    }

    func clipboardCopy(source: TerminalView, content: Data) {
        if let text = String(data: content, encoding: .utf8) {
            let pasteboard = NSPasteboard.general
            pasteboard.clearContents()
            pasteboard.setString(text, forType: .string)
        }
    }

    func rangeChanged(source: TerminalView, startY: Int, endY: Int) {
    }
}
