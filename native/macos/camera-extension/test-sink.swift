// Synthetic-only CMIO integration test. Never opens a physical camera.
// Build together with ../WebcamSink.swift; no AVFoundation capture session.
import Foundation
import CoreMediaIO
import CoreMedia
import CoreVideo
import ImageIO
import UniformTypeIdentifiers

private let targetUID="5C0B4E2A-1F3D-4A6B-9C7E-1D2E3F4A5B6C"
private func addr(_ selector:UInt32)->CMIOObjectPropertyAddress {CMIOObjectPropertyAddress(mSelector:selector,mScope:UInt32(kCMIOObjectPropertyScopeGlobal),mElement:UInt32(kCMIOObjectPropertyElementMain))}
private func ids(_ object:UInt32,_ selector:UInt32)->[UInt32]{var a=addr(selector);var n:UInt32=0;guard CMIOObjectGetPropertyDataSize(object,&a,0,nil,&n)==noErr,n>0,n<4096 else{return []};var result=[UInt32](repeating:0,count:Int(n)/4);var used:UInt32=0;guard CMIOObjectGetPropertyData(object,&a,0,nil,n,&used,&result)==noErr else{return []};return Array(result.prefix(Int(used)/4))}
private func uid(_ object:UInt32)->String?{var a=addr(UInt32(kCMIODevicePropertyDeviceUID));var value:Unmanaged<CFString>?;var used:UInt32=0;guard CMIOObjectGetPropertyData(object,&a,0,nil,UInt32(MemoryLayout<Unmanaged<CFString>?>.size),&used,&value)==noErr else{return nil};return value?.takeRetainedValue() as String?}
private struct TestFailure:Error{let message:String}
private func require(_ value:Bool,_ message:String)throws{if !value{throw TestFailure(message:message)}}
private func jpeg(red:Bool,width:Int,height:Int)throws->Data{
    var pixels=[UInt8](repeating:0,count:width*height*4)
    for i in stride(from:0,to:pixels.count,by:4){pixels[i]=red ? 0:255;pixels[i+1]=0;pixels[i+2]=red ? 255:0;pixels[i+3]=255}
    return try pixels.withUnsafeMutableBytes{raw in
        guard let context=CGContext(data:raw.baseAddress,width:width,height:height,bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGBitmapInfo.byteOrder32Little.rawValue|CGImageAlphaInfo.premultipliedFirst.rawValue),let image=context.makeImage() else{throw TestFailure(message:"synthetic JPEG image creation failed")}
        let data=NSMutableData()
        guard let destination=CGImageDestinationCreateWithData(data,UTType.jpeg.identifier as CFString,1,nil) else{throw TestFailure(message:"synthetic JPEG destination failed")}
        CGImageDestinationAddImage(destination,image,[kCGImageDestinationLossyCompressionQuality:0.95] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else{throw TestFailure(message:"synthetic JPEG encoding failed")}
        return data as Data
    }
}
private func meanChannels(_ pixel:CVPixelBuffer)->[Int]?{
    guard CVPixelBufferGetPixelFormatType(pixel)==kCVPixelFormatType_32BGRA else{return nil}
    CVPixelBufferLockBaseAddress(pixel,.readOnly);defer{CVPixelBufferUnlockBaseAddress(pixel,.readOnly)}
    guard let raw=CVPixelBufferGetBaseAddress(pixel)?.assumingMemoryBound(to:UInt8.self) else{return nil}
    let x=CVPixelBufferGetWidth(pixel)/2,y=CVPixelBufferGetHeight(pixel)/2,index=y*CVPixelBufferGetBytesPerRow(pixel)+x*4
    return [Int(raw[index+2]),Int(raw[index+1]),Int(raw[index])]
}
// Inspect only eight known standby-pattern sample points. No private pixel
// values are returned or logged. nil means an unsupported/unclassifiable buffer.
private func isKnownStandby(_ pixel:CVPixelBuffer)->Bool? {
    CVPixelBufferLockBaseAddress(pixel,.readOnly);defer{CVPixelBufferUnlockBaseAddress(pixel,.readOnly)}
    let format=CVPixelBufferGetPixelFormatType(pixel)
    if format==kCVPixelFormatType_32BGRA {
        guard let raw=CVPixelBufferGetBaseAddress(pixel)?.assumingMemoryBound(to:UInt8.self) else{return nil}
        let width=CVPixelBufferGetWidth(pixel),height=CVPixelBufferGetHeight(pixel),stride=CVPixelBufferGetBytesPerRow(pixel)
        // The provider's custom off-air artwork has a known dark border.
        // Inspect eight border points, never report private channel values.
        for (x,y) in [(width/8,height/8),(width/2,height/8),(width*7/8,height/8),(width/8,height/2),(width*7/8,height/2),(width/8,height*7/8),(width/2,height*7/8),(width*7/8,height*7/8)] {
            let at=y*stride+x*4
            for (channel,expected) in [35,21,15].enumerated(){if abs(Int(raw[at+channel])-expected)>5{return false}}
        }
        return true
    }
    if format==kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange || format==kCVPixelFormatType_420YpCbCr8BiPlanarFullRange {
        guard CVPixelBufferGetPlaneCount(pixel)==2,
              let y=CVPixelBufferGetBaseAddressOfPlane(pixel,0)?.assumingMemoryBound(to:UInt8.self),
              let uv=CVPixelBufferGetBaseAddressOfPlane(pixel,1)?.assumingMemoryBound(to:UInt8.self) else{return nil}
        let bands=[0x20,0x40,0x60,0x80,0xA0,0xC0,0xE0,0x20]
        let width=CVPixelBufferGetWidthOfPlane(pixel,0),height=CVPixelBufferGetHeightOfPlane(pixel,0)
        for i in 0..<8 {
            let row=(2*i+1)*height/16
            if abs(Int(y[row*CVPixelBufferGetBytesPerRowOfPlane(pixel,0)+width/2])-bands[i])>4{return false}
            let at=(row/2)*CVPixelBufferGetBytesPerRowOfPlane(pixel,1)+(width/2/2)*2
            if abs(Int(uv[at])-128)>4 || abs(Int(uv[at+1])-128)>4{return false}
        }
        return true
    }
    return nil
}
// Only enabled for known synthetic producer tests; never logs channel values.
private func syntheticColor(_ pixel:CVPixelBuffer)->String? {
    if CVPixelBufferGetPixelFormatType(pixel)==kCVPixelFormatType_32BGRA,let rgb=meanChannels(pixel) {
        if rgb[0]>180 && rgb[1]<70 && rgb[2]<70{return "red"}
        if rgb[2]>180 && rgb[0]<70 && rgb[1]<70{return "blue"}
        return nil
    }
    guard CVPixelBufferGetPlaneCount(pixel)==2 else{return nil}
    CVPixelBufferLockBaseAddress(pixel,.readOnly);defer{CVPixelBufferUnlockBaseAddress(pixel,.readOnly)}
    guard let base=CVPixelBufferGetBaseAddressOfPlane(pixel,1)?.assumingMemoryBound(to:UInt8.self) else{return nil}
    let row=CVPixelBufferGetHeightOfPlane(pixel,1)/2,col=(CVPixelBufferGetWidthOfPlane(pixel,1)/2)*2
    let offset=row*CVPixelBufferGetBytesPerRowOfPlane(pixel,1)+col
    let cb=Int(base[offset]),cr=Int(base[offset+1])
    if cr>180 && cb<150{return "red"}
    if cb>180 && cr<150{return "blue"}
    return nil
}
@main struct SinkTest {
    static func main(){
        var report:[String:Any]=["passed":false,"physical_camera_opened":false,"frames_saved":false,"device_uid":targetUID,"requested_fps":30]
        let launched=Date()
        let args=Array(CommandLine.arguments.dropFirst())
        let consumeOnly=args.contains("--consume-only")
        let requireLive=args.contains("--require-live")
        let syntheticColors=args.contains("--synthetic-colors")
        func trace(_ stage:String){if args.contains("--trace"){FileHandle.standardError.write(Data((stage+"\n").utf8))}}
        func option(_ name:String)->String? {args.firstIndex(of:name).flatMap{index in args.indices.contains(index+1) ? args[index+1] : nil}}
        let output=option("--output") ?? args.first(where:{!$0.hasPrefix("--") && ![option("--frames"),option("--timeout"),option("--quality"),option("--pixel-format"),option("--format-sequence"),option("--ready-file")].compactMap{$0}.contains($0)}) ?? "webcam-macos-sink.json"
        let quality=option("--quality") ?? "1080p"
        let width=quality=="720p" ? 1280:1920,height=quality=="720p" ? 720:1080
        let pixelName=option("--pixel-format") ?? "bgra"
        let pixelFormat=pixelName=="nv12" ? kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange:kCVPixelFormatType_32BGRA
        report["requested_pixel_format"]=pixelFormat
        report["quality"]=quality;report["requested_width"]=width;report["requested_height"]=height
        let target=min(max(Int(option("--frames") ?? "60") ?? 60,1),300)
        let sequence=(option("--format-sequence") ?? quality).split(separator:",").map(String.init)
        let totalTarget=target*sequence.count
        report["format_sequence"]=sequence
        let timeout=min(max(Double(option("--timeout") ?? "8") ?? 8,1),25)
        report["mode"]=consumeOnly ? "consume-only" : "synthetic-producer-and-consumer"
        report["require_live"]=requireLive
        report["private_pixel_values_logged"]=false
        report["require_synthetic_colors"]=syntheticColors
        report["requested_samples"]=consumeOnly ? target : 2

        var source:UInt32=0,device:UInt32=0
        defer{
            trace("teardown-start");ponteCameraSinkStop()
            if source != 0 {report["source_stop_osstatus"]=CMIODeviceStopStream(device,source)}
            if let pointer=ponteCameraSinkStatusJSON(){
                let data=Data(bytes:pointer,count:strlen(pointer));free(pointer)
                if let status=try? JSONSerialization.jsonObject(with:data) as? [String:Any] {report["sink_active_after_stop"]=status["active"]}
            }
            trace("teardown-finished");report["teardown_called"]=true
            if let bytes=try? JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]){try? bytes.write(to:URL(fileURLWithPath:output),options:.atomic);print(String(data:bytes,encoding:.utf8)!)}
            exit((report["passed"] as? Bool)==true ? 0:1)
        }
        do{
            try require(!requireLive || consumeOnly,"--require-live needs --consume-only")
            try require(["720p","1080p"].contains(quality),"unsupported quality")
            try require(["bgra","nv12"].contains(pixelName),"unsupported pixel format")
            try require(sequence.count<=3 && sequence.allSatisfy{["720p","1080p"].contains($0)} && sequence.first==quality,"invalid format sequence")
            try require(sequence.count==1 || (consumeOnly && requireLive),"format sequence requires live consumer")
            // This process opts into extension devices; no screen or physical
            // camera pixels are requested by this metadata-only property.
            var allow:UInt32=1;var allowAddress=addr(UInt32(kCMIOHardwarePropertyAllowScreenCaptureDevices))
            report["extension_device_visibility_osstatus"]=CMIOObjectSetPropertyData(UInt32(kCMIOObjectSystemObject),&allowAddress,0,nil,4,&allow)
            let discoveryDeadline=Date().addingTimeInterval(3)
            repeat{
                device=ids(UInt32(kCMIOObjectSystemObject),UInt32(kCMIOHardwarePropertyDevices)).first{uid($0)==targetUID} ?? 0
                if device != 0{break}
                CFRunLoopRunInMode(.defaultMode,0.05,false)
            }while Date()<discoveryDeadline
            trace("discovery-finished");report["device_found"]=device != 0
            try require(device != 0,"enabled virtual device not found in CMIO")
            let streams=ids(device,UInt32(kCMIODevicePropertyStreams));report["stream_count"]=streams.count
            try require(streams.count==2,"virtual source+sink stream pair missing");source=streams[0]
            var formatAddress=addr(UInt32(kCMIOStreamPropertyFormatDescriptions));var formats:Unmanaged<CFArray>?;var used:UInt32=0
            let listStatus=CMIOObjectGetPropertyData(source,&formatAddress,0,nil,UInt32(MemoryLayout<Unmanaged<CFArray>?>.size),&used,&formats)
            trace("format-list-finished");report["format_list_osstatus"]=listStatus
            let formatList=listStatus==noErr ? formats?.takeRetainedValue() as? [CMFormatDescription] : nil
            func selectFormat(_ selectedQuality:String)throws->OSStatus {
                let w=selectedQuality=="720p" ? 1280:1920,h=selectedQuality=="720p" ? 720:1080
                guard let list=formatList,let selected=list.first(where:{CMVideoFormatDescriptionGetDimensions($0).width==Int32(w) && CMVideoFormatDescriptionGetDimensions($0).height==Int32(h) && CMFormatDescriptionGetMediaSubType($0)==pixelFormat}) else{throw TestFailure(message:"consumer format unavailable")}
                var pointer:UnsafeMutableRawPointer?=Unmanaged.passUnretained(selected).toOpaque();var address=addr(UInt32(kCMIOStreamPropertyFormatDescription))
                let result=CMIOObjectSetPropertyData(source,&address,0,nil,UInt32(MemoryLayout<UnsafeMutableRawPointer?>.size),&pointer)
                try require(result==noErr,"consumer source format setter failed: \(result)")
                return result
            }
            if !args.contains("--keep-source-format"){report["select_source_format_osstatus"]=try selectFormat(quality)}
            var unretained:Unmanaged<CMSimpleQueue>?
            trace("source-queue-start");let queueResult=CMIOStreamCopyBufferQueue(source,{_,_,_ in},nil,&unretained);report["source_queue_osstatus"]=queueResult
            try require(queueResult==noErr && unretained != nil,"source CMIO queue unavailable")
            let queue=unretained!.takeRetainedValue()
            trace("source-stream-start");let startResult=CMIODeviceStartStream(device,source);report["source_start_osstatus"]=startResult
            var activeAddress=addr(UInt32(kCMIOStreamPropertyFormatDescription));var activeDescription:Unmanaged<CMFormatDescription>?;var activeUsed:UInt32=0
            let activeResult=CMIOObjectGetPropertyData(source,&activeAddress,0,nil,UInt32(MemoryLayout<Unmanaged<CMFormatDescription>?>.size),&activeUsed,&activeDescription)
            if activeResult==noErr,let active=activeDescription?.takeRetainedValue(){let dims=CMVideoFormatDescriptionGetDimensions(active);report["source_format_after_start"]=["width":dims.width,"height":dims.height,"pixel_format":CMFormatDescriptionGetMediaSubType(active)];trace("source-format-after-start \(dims.width)x\(dims.height) pixel=\(CMFormatDescriptionGetMediaSubType(active))")}
            trace("source-stream-start-finished");try require(startResult==noErr,"virtual source stream refused start")
            report["setup_elapsed_seconds"]=Date().timeIntervalSince(launched)
            if let ready=option("--ready-file"){try Data("ready \(ProcessInfo.processInfo.processIdentifier)\n".utf8).write(to:URL(fileURLWithPath:ready),options:.atomic)}
            if !consumeOnly {
                let sinkResult=ponteCameraSinkStart(UInt32(width),UInt32(height),30);report["sink_start_code"]=sinkResult
                if let pointer=ponteCameraSinkStatusJSON(){report["sink_status"]=String(cString:pointer);free(pointer)}
                try require(sinkResult==0,"Ponte Desk sink refused start")
            }
            let red=consumeOnly ? Data() : try jpeg(red:true,width:width,height:height),blue=consumeOnly ? Data() : try jpeg(red:false,width:width,height:height)
            let deadline=Date().addingTimeInterval(timeout);let began=Date();var sent=0,received=0,live=0,standby=0,unknown=0,redSeen=false,blueSeen=false;var colors:[[Int]]=[];var dimensions:Set<String>=[];var liveDimensions:Set<String>=[];var unexpectedSynthetic=0;var wrongSizeLive=0;var wrongFormatLive=0;var pixelFormats:Set<UInt32>=[];var lastTrace=Date()
            var stage=0,stageLive=0,expectedWidth=width,expectedHeight=height;var stages:[[String:Any]]=[]
            while Date()<deadline && (consumeOnly ? (requireLive ? live<totalTarget:received<target) : !(redSeen && blueSeen)){
                if Date().timeIntervalSince(lastTrace)>1{trace("sampling received=\(received) live=\(live) elapsed=\(Date().timeIntervalSince(began))");lastTrace=Date()}
                if !consumeOnly {
                    let next=redSeen ? blue:red
                    let push=next.withUnsafeBytes{ponteCameraSinkPushJPEG($0.bindMemory(to:UInt8.self).baseAddress,UInt32(next.count))}
                    try require(push>=0,"JPEG sink push failed: \(push)");sent+=1
                }
                CFRunLoopRunInMode(.defaultMode,consumeOnly ? 0.01 : 1.0/30,false)
                while let element=CMSimpleQueueDequeue(queue){
                    let sample=Unmanaged<CMSampleBuffer>.fromOpaque(element).takeRetainedValue();received+=1
                    if let pixel=CMSampleBufferGetImageBuffer(sample){
                        dimensions.insert("\(CVPixelBufferGetWidth(pixel))x\(CVPixelBufferGetHeight(pixel))");pixelFormats.insert(CVPixelBufferGetPixelFormatType(pixel))
                        if consumeOnly && requireLive {
                            if let known=isKnownStandby(pixel){
                                if known{standby+=1}else{
                                    let size="\(CVPixelBufferGetWidth(pixel))x\(CVPixelBufferGetHeight(pixel))"
                                    liveDimensions.insert(size)
                                    if CVPixelBufferGetWidth(pixel)==expectedWidth && CVPixelBufferGetHeight(pixel)==expectedHeight {
                                        if CVPixelBufferGetPixelFormatType(pixel)==pixelFormat {
                                            let color=syntheticColors ? syntheticColor(pixel):nil
                                            if color=="red"{redSeen=true};if color=="blue"{blueSeen=true}
                                            if !syntheticColors || color != nil {live+=1;stageLive+=1}else{unexpectedSynthetic+=1}
                                            if stageLive==target {
                                                stages.append(["quality":sequence[stage],"live_samples":stageLive,"width":expectedWidth,"height":expectedHeight,"pixel_format":pixelFormat])
                                                if stage+1<sequence.count {
                                                    stage+=1;stageLive=0;expectedWidth=sequence[stage]=="720p" ? 1280:1920;expectedHeight=sequence[stage]=="720p" ? 720:1080
                                                    let status=try selectFormat(sequence[stage]);trace("consumer-negotiated \(sequence[stage]) status=\(status)")
                                                }
                                            }
                                        }else{wrongFormatLive+=1}
                                    }else{wrongSizeLive+=1}
                                }
                            }else{unknown+=1}
                        }
                        if !consumeOnly,let rgb=meanChannels(pixel){
                            if colors.count<12{colors.append(rgb)}
                            if rgb[0]>180 && rgb[1]<70 && rgb[2]<70{redSeen=true}
                            if rgb[2]>180 && rgb[0]<70 && rgb[1]<70{blueSeen=true}
                        }
                    }
                }
            }
            trace("sampling-finished");report["pushed_frames"]=sent;report["received_samples"]=received;report["observed_dimensions"]=dimensions.sorted();report["sampling_elapsed_seconds"]=Date().timeIntervalSince(began)
            if !consumeOnly {report["red_seen"]=redSeen;report["blue_seen"]=blueSeen;report["sample_center_rgb"]=colors;try require(redSeen && blueSeen,"synthetic red and blue frames were not received")}
            else {
                if syntheticColors{report["synthetic_red_seen"]=redSeen;report["synthetic_blue_seen"]=blueSeen;report["unexpected_synthetic_samples"]=unexpectedSynthetic;try require(redSeen && blueSeen,"synthetic colors missing")}
                report["format_stages"]=stages;report["live_samples"]=live;report["standby_samples"]=standby;report["unclassified_samples"]=unknown;report["wrong_size_live_samples"]=wrongSizeLive;report["wrong_format_live_samples"]=wrongFormatLive;report["observed_pixel_formats"]=pixelFormats.sorted();report["live_dimensions"]=liveDimensions.sorted()
                try require((requireLive ? live:received)>=(requireLive ? totalTarget:target),requireLive ? "live sample count below requested minimum (standby rejected)":"source sample count below requested minimum")
            }
            try require(dimensions.contains("\(width)x\(height)"),"wrong frame dimensions")
            report["passed"]=true
        }catch let error as TestFailure{report["error"]=error.message}catch{report["error"]=error.localizedDescription}
    }
}
