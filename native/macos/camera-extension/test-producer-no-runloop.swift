// Synthetic producer runs on a plain background Thread. The process main
// thread waits on a semaphore; neither application thread pumps CFRunLoop.
import Foundation
import CoreGraphics
import CoreMediaIO
import CoreMedia
import ImageIO
import UniformTypeIdentifiers
private final class TestResult: @unchecked Sendable {
    private let lock=NSLock();private var value:[String:Any]=[:]
    func set(_ next:[String:Any]){lock.lock();value=next;lock.unlock()}
    func get()->[String:Any]{lock.lock();defer{lock.unlock()};return value}
}
private struct ProducerFailure:Error{let text:String}
private func makeJPEG(red:Bool,width:Int,height:Int)throws->Data{
    var pixels=[UInt8](repeating:0,count:width*height*4)
    for offset in stride(from:0,to:pixels.count,by:4){pixels[offset]=red ? 0:255;pixels[offset+1]=0;pixels[offset+2]=red ? 255:0;pixels[offset+3]=255}
    return try pixels.withUnsafeMutableBytes{raw in
        guard let context=CGContext(data:raw.baseAddress,width:width,height:height,bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGBitmapInfo.byteOrder32Little.rawValue|CGImageAlphaInfo.premultipliedFirst.rawValue),let image=context.makeImage() else{throw ProducerFailure(text:"synthetic image unavailable")}
        let data=NSMutableData()
        guard let destination=CGImageDestinationCreateWithData(data,UTType.jpeg.identifier as CFString,1,nil) else{throw ProducerFailure(text:"synthetic JPEG unavailable")}
        CGImageDestinationAddImage(destination,image,[kCGImageDestinationLossyCompressionQuality:0.9] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else{throw ProducerFailure(text:"synthetic JPEG encode failed")}
        return data as Data
    }
}
private func sourceFormatSnapshot()->[String:Any] {
    func address(_ selector:UInt32)->CMIOObjectPropertyAddress{CMIOObjectPropertyAddress(mSelector:selector,mScope:UInt32(kCMIOObjectPropertyScopeGlobal),mElement:UInt32(kCMIOObjectPropertyElementMain))}
    func ids(_ object:UInt32,_ selector:UInt32)->[UInt32]{var a=address(selector);var size:UInt32=0;guard CMIOObjectGetPropertyDataSize(object,&a,0,nil,&size)==noErr,size>0,size<4096 else{return []};var result=[UInt32](repeating:0,count:Int(size)/4);var used:UInt32=0;guard CMIOObjectGetPropertyData(object,&a,0,nil,size,&used,&result)==noErr else{return []};return result}
    for device in ids(UInt32(kCMIOObjectSystemObject),UInt32(kCMIOHardwarePropertyDevices)) {
        var a=address(UInt32(kCMIODevicePropertyDeviceUID));var uid:Unmanaged<CFString>?;var used:UInt32=0
        guard CMIOObjectGetPropertyData(device,&a,0,nil,UInt32(MemoryLayout<Unmanaged<CFString>?>.size),&used,&uid)==noErr,let actual=uid?.takeRetainedValue() as String?,actual=="5C0B4E2A-1F3D-4A6B-9C7E-1D2E3F4A5B6C" else{continue}
        guard let source=ids(device,UInt32(kCMIODevicePropertyStreams)).first else{return ["error":"source not found"]}
        a=address(UInt32(kCMIOStreamPropertyFormatDescription));var format:Unmanaged<CMFormatDescription>?
        let status=CMIOObjectGetPropertyData(source,&a,0,nil,UInt32(MemoryLayout<Unmanaged<CMFormatDescription>?>.size),&used,&format)
        guard status==noErr,let format=format?.takeRetainedValue() else{return ["getter_osstatus":status]}
        let dimensions=CMVideoFormatDescriptionGetDimensions(format)
        return ["getter_osstatus":status,"width":dimensions.width,"height":dimensions.height,"pixel_format":CMFormatDescriptionGetMediaSubType(format)]
    }
    return ["error":"virtual device not found"]
}
@main struct ProducerNoRunLoop {
    static func main(){
        let args=Array(CommandLine.arguments.dropFirst())
        func option(_ name:String)->String? {args.firstIndex(of:name).flatMap{index in args.indices.contains(index+1) ? args[index+1] : nil}}
        let quality=option("--quality") ?? "1080p"
        let width=quality=="720p" ? 1280:1920,height=quality=="720p" ? 720:1080
        let pump=args.contains("--pump-producer")
        let duration=min(max(Double(option("--duration") ?? "3") ?? 3,1),10)
        let output=option("--output") ?? args.first(where:{!$0.hasPrefix("--") && $0 != quality}) ?? "webcam-macos-no-runloop-producer.json"
        let done=DispatchSemaphore(value:0);let result=TestResult()
        Thread.detachNewThread{
            var report:[String:Any]=["passed":false,"physical_camera_opened":false,"frames_saved":false,"producer_on_main_thread":Thread.isMainThread,"application_cf_runloop_pumped":pump,"requested_duration_seconds":duration,"patterns":"synthetic red/blue","quality":quality,"width":width,"height":height]
            defer{
                ponteCameraSinkStop()
                if let pointer=ponteCameraSinkStatusJSON(){let data=Data(bytes:pointer,count:strlen(pointer));free(pointer);if let status=try? JSONSerialization.jsonObject(with:data) as? [String:Any]{report["sink_active_after_stop"]=status["active"]}}
                result.set(report);done.signal()
            }
            do{
                guard ["720p","1080p"].contains(quality) else{throw ProducerFailure(text:"unsupported quality")}
                report["source_format_before_sink_start"]=sourceFormatSnapshot()
                let start=ponteCameraSinkStart(UInt32(width),UInt32(height),30);report["sink_start_code"]=start
                report["source_format_after_sink_start"]=sourceFormatSnapshot()
                if start != 0 {throw ProducerFailure(text:"sink start failed (\(start))")}
                let red=try makeJPEG(red:true,width:width,height:height),blue=try makeJPEG(red:false,width:width,height:height)
                let began=Date();let deadline=began.addingTimeInterval(duration)
                var attempts=0,enqueued=0,dropped=0
                while Date()<deadline {
                    if pump{CFRunLoopRunInMode(.defaultMode,0.001,true)}
                    let frame=(attempts/15).isMultiple(of:2) ? red:blue
                    let push=frame.withUnsafeBytes{ponteCameraSinkPushJPEG($0.bindMemory(to:UInt8.self).baseAddress,UInt32(frame.count))}
                    if push<0{throw ProducerFailure(text:"sink push failed (\(push))")}
                    if push==0{enqueued+=1}else{dropped+=1};attempts+=1
                    Thread.sleep(forTimeInterval:1.0/30)
                }
                report["source_format_after_pushes"]=sourceFormatSnapshot()
                if let pointer=ponteCameraSinkStatusJSON(){let data=Data(bytes:pointer,count:strlen(pointer));free(pointer);report["sink_status_before_stop"]=try? JSONSerialization.jsonObject(with:data)}
                report["attempts"]=attempts;report["enqueued"]=enqueued;report["dropped_by_backpressure"]=dropped;report["elapsed_seconds"]=Date().timeIntervalSince(began);report["passed"]=enqueued>0
            }catch let error as ProducerFailure{report["error"]=error.text}catch{report["error"]=error.localizedDescription}
        }
        let finished=done.wait(timeout:.now()+duration+5) == .success
        let report=finished ? result.get():["passed":false,"error":"producer process exceeded 8 seconds","physical_camera_opened":false]
        if let data=try? JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]){try? data.write(to:URL(fileURLWithPath:output),options:.atomic);print(String(data:data,encoding:.utf8)!)}
        exit((report["passed"] as? Bool)==true ? 0:1)
    }
}
