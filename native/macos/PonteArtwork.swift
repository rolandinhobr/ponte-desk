// Ponte Desk artwork. GPL-3.0-or-later.
import AppKit

enum PonteArtwork {
    private static func drawConnection(in rect: NSRect, paused: Bool) {
        let scale = min(rect.width, rect.height) / 18
        let transform = AffineTransform(translationByX: rect.midX - 9 * scale,
                                        byY: rect.midY - 9 * scale)
        var scaled = transform
        scaled.scale(scale)
        let left = NSBezierPath()
        left.move(to: NSPoint(x: 7, y: 13.5))
        left.line(to: NSPoint(x: 4.5, y: 13.5))
        left.curve(to: NSPoint(x: 2.5, y: 11.5), controlPoint1: NSPoint(x: 3.3, y: 13.5), controlPoint2: NSPoint(x: 2.5, y: 12.7))
        left.line(to: NSPoint(x: 2.5, y: 6.5))
        left.curve(to: NSPoint(x: 4.5, y: 4.5), controlPoint1: NSPoint(x: 2.5, y: 5.3), controlPoint2: NSPoint(x: 3.3, y: 4.5))
        left.line(to: NSPoint(x: 7, y: 4.5))
        let right = NSBezierPath()
        right.move(to: NSPoint(x: 11, y: 13.5))
        right.line(to: NSPoint(x: 13.5, y: 13.5))
        right.curve(to: NSPoint(x: 15.5, y: 11.5), controlPoint1: NSPoint(x: 14.7, y: 13.5), controlPoint2: NSPoint(x: 15.5, y: 12.7))
        right.line(to: NSPoint(x: 15.5, y: 6.5))
        right.curve(to: NSPoint(x: 13.5, y: 4.5), controlPoint1: NSPoint(x: 15.5, y: 5.3), controlPoint2: NSPoint(x: 14.7, y: 4.5))
        right.line(to: NSPoint(x: 11, y: 4.5))
        let bridge = NSBezierPath()
        bridge.move(to: NSPoint(x: 6, y: 9))
        bridge.line(to: NSPoint(x: paused ? 7.5 : 12, y: 9))
        if paused {
            bridge.move(to: NSPoint(x: 10.5, y: 9))
            bridge.line(to: NSPoint(x: 12, y: 9))
        }
        for path in [left, right, bridge] {
            path.transform(using: scaled)
            path.lineWidth = 1.5 * scale
            path.lineCapStyle = .round
            path.lineJoinStyle = .round
            path.stroke()
        }
    }

    static func menuIcon(paused: Bool = false) -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { rect in
            NSColor.black.setStroke()
            drawConnection(in: rect, paused: paused)
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = paused ? "Ponte Desk — compartilhamento pausado" : "Ponte Desk"
        return image
    }

    static func badge(size: CGFloat) -> NSImage {
        let image = NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            NSColor.systemIndigo.withAlphaComponent(0.11).setFill()
            NSBezierPath(roundedRect: rect.insetBy(dx: 0.5, dy: 0.5),
                         xRadius: size * 0.25, yRadius: size * 0.25).fill()
            NSColor.systemIndigo.setStroke()
            drawConnection(in: rect.insetBy(dx: size * 0.16, dy: size * 0.16), paused: false)
            return true
        }
        image.accessibilityDescription = "Ponte Desk"
        return image
    }
}
