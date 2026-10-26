import AppKit
@main struct RenderIcon {
    static func main() throws {
        guard CommandLine.arguments.count==2 else{return}
        let root=URL(fileURLWithPath:CommandLine.arguments[1]);try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true)
        for base in [16,32,128,256,512] {
            for scale in [1,2] {
                let pixels=base*scale
                guard let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:pixels,pixelsHigh:pixels,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:0,bitsPerPixel:0),let context=NSGraphicsContext(bitmapImageRep:bitmap) else{continue}
                NSGraphicsContext.saveGraphicsState();NSGraphicsContext.current=context
                NSAppearance(named:.aqua)?.performAsCurrentDrawingAppearance {PonteArtwork.badge(size:CGFloat(pixels)).draw(in:NSRect(x:0,y:0,width:pixels,height:pixels))}
                NSGraphicsContext.restoreGraphicsState()
                let suffix=scale==2 ? "@2x":""
                try bitmap.representation(using:.png,properties:[:])?.write(to:root.appendingPathComponent("icon_\(base)x\(base)\(suffix).png"))
            }
        }
    }
}
