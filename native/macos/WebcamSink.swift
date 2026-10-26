// Ponte Desk CMIO sink. GPL-3.0-or-later.
// Uses the public CMIO sink protocol documented by VirtualCamera4Mac
// (Apache-2.0), m96-chan, https://github.com/m96-chan/VirtualCamera4Mac.
// Bounded JPEG decoding follows the format negotiated by the consuming app.
// This producer never changes source-stream properties or installs extensions.
import Foundation
import CoreMediaIO
import CoreMedia
import CoreVideo
import ImageIO
import VideoToolbox

private let ponteDefaultCameraUID = "5C0B4E2A-1F3D-4A6B-9C7E-1D2E3F4A5B6C"
private let ponteSinkLock = NSLock()
private var ponteSink: PonteCameraSink?
private var ponteSinkMessage = "Extensão de câmera ainda não conectada"

private func cmioAddress(_ selector: UInt32) -> CMIOObjectPropertyAddress {
    CMIOObjectPropertyAddress(mSelector: selector,
                             mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeGlobal),
                             mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementMain))
}
private func cmioIDs(_ object: CMIOObjectID, _ selector: UInt32) -> [UInt32] {
    var address=cmioAddress(selector);var length:UInt32=0
    guard CMIOObjectGetPropertyDataSize(object,&address,0,nil,&length)==noErr,
          length>0,length<=4096,length % 4 == 0 else{return []}
    var ids=[UInt32](repeating:0,count:Int(length)/4);var used:UInt32=0
    guard CMIOObjectGetPropertyData(object,&address,0,nil,length,&used,&ids)==noErr else{return []}
    return Array(ids.prefix(Int(used)/4))
}
private func cmioUID(_ object:CMIOObjectID)->String? {
    var address=cmioAddress(UInt32(kCMIODevicePropertyDeviceUID));var value:Unmanaged<CFString>?;var used:UInt32=0
    guard CMIOObjectGetPropertyData(object,&address,0,nil,UInt32(MemoryLayout<Unmanaged<CFString>?>.size),&used,&value)==noErr else{return nil}
    return value?.takeRetainedValue() as String?
}
private func ponteVirtualDevice()->CMIOObjectID? {
    let uid=ponteDefaultCameraUID // Do not let an arbitrary physical device become a sink.
    return cmioIDs(CMIOObjectID(kCMIOObjectSystemObject),UInt32(kCMIOHardwarePropertyDevices)).first{cmioUID($0)?.caseInsensitiveCompare(uid) == .orderedSame}
}
private struct SinkFailure: Error {let code:Int32;let message:String}

private final class SinkFormatChanges {
    private let lock=NSLock();private var value=0
    func increment(){lock.lock();value+=1;lock.unlock()}
    func count()->Int{lock.lock();defer{lock.unlock()};return value}
}
private struct SinkOutput: Equatable {
    let width:Int;let height:Int;let pixelFormat:OSType
}
private func negotiatedOutput(_ stream:CMIOStreamID)throws->SinkOutput {
    var address=cmioAddress(UInt32(kCMIOStreamPropertyFormatDescription))
    var description:Unmanaged<CMFormatDescription>?;var used:UInt32=0
    let result=CMIOObjectGetPropertyData(stream,&address,0,nil,UInt32(MemoryLayout<Unmanaged<CMFormatDescription>?>.size),&used,&description)
    guard result==noErr,let format=description?.takeRetainedValue() else{throw SinkFailure(code:-4,message:"Formato negociado da câmera indisponível (\(result))")}
    let dimensions=CMVideoFormatDescriptionGetDimensions(format),pixel=CMFormatDescriptionGetMediaSubType(format)
    guard [(1280,720),(1920,1080)].contains(where:{$0.0==Int(dimensions.width) && $0.1==Int(dimensions.height)}),
          [kCVPixelFormatType_32BGRA,kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,kCVPixelFormatType_420YpCbCr8BiPlanarFullRange].contains(pixel) else{throw SinkFailure(code:-3,message:"O aplicativo selecionou um formato de câmera não suportado")}
    return SinkOutput(width:Int(dimensions.width),height:Int(dimensions.height),pixelFormat:pixel)
}
private final class PonteCameraSink {
    let width:Int;let height:Int;let fps:Int32
    private let device:CMIOObjectID
    private let sourceStream:CMIOStreamID
    private let stream:CMIOStreamID
    private let queue:CMSimpleQueue
    private var output:SinkOutput?
    private var bgraPool:CVPixelBufferPool?
    private var nv12Pool:CVPixelBufferPool?
    private var transfer:VTPixelTransferSession?
    private var format:CMVideoFormatDescription?
    private var stopped=false
    private let propertyQueue=DispatchQueue(label:"io.pontedesk.camera.source-format")
    private var formatListener:CMIOObjectPropertyListenerBlock?
    private let formatChanges=SinkFormatChanges()
    var metadata:[String:Any]{var values:[String:Any]=["source_format_notifications":formatChanges.count()];if let output{values["output_width"]=output.width;values["output_height"]=output.height;values["output_pixel_format"]=output.pixelFormat};return values}
    init(width:Int,height:Int,fps:Int32)throws {
        guard [(1280,720),(1920,1080)].contains(where:{$0.0==width && $0.1==height}),fps==30 else{throw SinkFailure(code:-3,message:"Formato de câmera não suportado")}
        guard let device=ponteVirtualDevice() else{throw SinkFailure(code:-2,message:"Configure e ative a câmera virtual do Ponte Desk")}
        let streams=cmioIDs(device,UInt32(kCMIODevicePropertyStreams))
        guard streams.count==2 else{throw SinkFailure(code:-4,message:"A câmera não publicou o stream de entrada esperado")}
        self.width=width;self.height=height;self.fps=fps;self.device=device;self.sourceStream=streams[0];self.stream=streams[1]
        var unmanaged:Unmanaged<CMSimpleQueue>?
        guard CMIOStreamCopyBufferQueue(stream,{_,_,_ in},nil,&unmanaged)==noErr,let unmanaged else{throw SinkFailure(code:-4,message:"Fila de entrada da câmera indisponível")}
        queue=unmanaged.takeRetainedValue()
        // Register a dispatch listener so CMIO updates its source-format cache
        // when another client negotiates, without a main-thread CFRunLoop.
        let changes=formatChanges
        let listener:CMIOObjectPropertyListenerBlock={_,_ in changes.increment()}
        var property=cmioAddress(UInt32(kCMIOStreamPropertyFormatDescription))
        let listenerResult=CMIOObjectAddPropertyListenerBlock(sourceStream,&property,propertyQueue,listener)
        guard listenerResult==noErr else{throw SinkFailure(code:-4,message:"Notificação de formato indisponível (\(listenerResult))")}
        formatListener=listener
        try configure(try negotiatedOutput(sourceStream))
        let result=CMIODeviceStartStream(device,stream)
        guard result==noErr else{throw SinkFailure(code:-4,message:"Não foi possível iniciar a câmera virtual (\(result))")}
    }
    private func makePool(_ output:SinkOutput,pixelFormat:OSType)throws->CVPixelBufferPool {
        let attributes:[String:Any]=[kCVPixelBufferWidthKey as String:output.width,
            kCVPixelBufferHeightKey as String:output.height,
            kCVPixelBufferPixelFormatTypeKey as String:pixelFormat,
            kCVPixelBufferCGImageCompatibilityKey as String:pixelFormat==kCVPixelFormatType_32BGRA,
            kCVPixelBufferCGBitmapContextCompatibilityKey as String:pixelFormat==kCVPixelFormatType_32BGRA,
            kCVPixelBufferIOSurfacePropertiesKey as String:[:]]
        var created:CVPixelBufferPool?
        guard CVPixelBufferPoolCreate(nil,nil,attributes as CFDictionary,&created)==kCVReturnSuccess,let created else{throw SinkFailure(code:-5,message:"Não foi possível criar buffers da câmera")}
        return created
    }
    private func configure(_ next:SinkOutput)throws {
        guard next != output else{return}
        // No private backlog: only the current bounded pools are retained.
        // Reconfigure only at an empty sink queue, before decoding another JPEG.
        let bgra=try makePool(next,pixelFormat:kCVPixelFormatType_32BGRA)
        let nv12=next.pixelFormat==kCVPixelFormatType_32BGRA ? nil:try makePool(next,pixelFormat:next.pixelFormat)
        if nv12 != nil && transfer==nil {
            var created:VTPixelTransferSession?
            let result=VTPixelTransferSessionCreate(allocator:nil,pixelTransferSessionOut:&created)
            guard result==noErr,let created else{throw SinkFailure(code:-5,message:"Conversão NV12 indisponível (\(result))")}
            transfer=created
        }
        bgraPool=bgra;nv12Pool=nv12;format=nil;output=next
    }
    private func allocate(_ pool:CVPixelBufferPool)throws->CVPixelBuffer? {
        var pixel:CVPixelBuffer?
        let result=CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(nil,pool,[kCVPixelBufferPoolAllocationThresholdKey:3] as CFDictionary,&pixel)
        if result==kCVReturnWouldExceedAllocationThreshold{return nil}
        guard result==kCVReturnSuccess,let pixel else{throw SinkFailure(code:-5,message:"Buffer da câmera indisponível")}
        return pixel
    }
    func stop(){
        if let listener=formatListener{var property=cmioAddress(UInt32(kCMIOStreamPropertyFormatDescription));_ = CMIOObjectRemovePropertyListenerBlock(sourceStream,&property,propertyQueue,listener);formatListener=nil}
        if !stopped{_ = CMIODeviceStopStream(device,stream);stopped=true}
        if let transfer{VTPixelTransferSessionInvalidate(transfer)}
        transfer=nil;bgraPool=nil;nv12Pool=nil;format=nil;output=nil
    }
    deinit{stop()}
    func push(_ data:Data)throws->Int32 {
        guard !stopped else{throw SinkFailure(code:-4,message:"A câmera virtual está desligada")}
        guard CMSimpleQueueGetCount(queue)<CMSimpleQueueGetCapacity(queue) else{return 1}
        // The consuming app owns source-format negotiation. Read it on frame
        // boundaries instead of forcing a format on another application's stream.
        try configure(try negotiatedOutput(sourceStream))
        guard let output,let bgraPool else{throw SinkFailure(code:-5,message:"Buffers da câmera indisponíveis")}
        guard data.count<=2*1024*1024,let source=CGImageSourceCreateWithData(data as CFData,nil),
              let properties=CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],
              properties[kCGImagePropertyPixelWidth] as? Int==width,
              properties[kCGImagePropertyPixelHeight] as? Int==height,
              let image=CGImageSourceCreateImageAtIndex(source,0,[kCGImageSourceShouldCacheImmediately:true] as CFDictionary) else{throw SinkFailure(code:-5,message:"JPEG inválido ou dimensões diferentes do stream")}
        guard let bgra=try allocate(bgraPool) else{return 1}
        let lock=CVPixelBufferLockBaseAddress(bgra,[])
        guard lock==kCVReturnSuccess else{throw SinkFailure(code:-5,message:"Buffer BGRA indisponível")}
        do {
            defer{CVPixelBufferUnlockBaseAddress(bgra,[])}
            guard let base=CVPixelBufferGetBaseAddress(bgra),let context=CGContext(data:base,width:output.width,height:output.height,bitsPerComponent:8,bytesPerRow:CVPixelBufferGetBytesPerRow(bgra),space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGBitmapInfo.byteOrder32Little.rawValue|CGImageAlphaInfo.premultipliedFirst.rawValue) else{throw SinkFailure(code:-5,message:"Conversão BGRA indisponível")}
            let scale=min(CGFloat(output.width)/CGFloat(width),CGFloat(output.height)/CGFloat(height))
            let drawnWidth=CGFloat(width)*scale,drawnHeight=CGFloat(height)*scale
            context.setFillColor(CGColor(gray:0,alpha:1));context.fill(CGRect(x:0,y:0,width:output.width,height:output.height))
            context.interpolationQuality = .medium
            context.draw(image,in:CGRect(x:(CGFloat(output.width)-drawnWidth)/2,y:(CGFloat(output.height)-drawnHeight)/2,width:drawnWidth,height:drawnHeight))
        }
        let pixel:CVPixelBuffer
        if let nv12Pool {
            guard let converted=try allocate(nv12Pool) else{return 1}
            guard let transfer,VTPixelTransferSessionTransferImage(transfer,from:bgra,to:converted)==noErr else{throw SinkFailure(code:-5,message:"Conversão BGRA para NV12 falhou")}
            pixel=converted
        }else{pixel=bgra}
        if format==nil{CMVideoFormatDescriptionCreateForImageBuffer(allocator:nil,imageBuffer:pixel,formatDescriptionOut:&format)}
        guard let format else{throw SinkFailure(code:-5,message:"Descrição de vídeo indisponível")}
        var timing=CMSampleTimingInfo(duration:CMTime(value:1,timescale:fps),presentationTimeStamp:CMClockGetTime(CMClockGetHostTimeClock()),decodeTimeStamp:.invalid)
        var sample:CMSampleBuffer?
        guard CMSampleBufferCreateForImageBuffer(allocator:nil,imageBuffer:pixel,dataReady:true,makeDataReadyCallback:nil,refcon:nil,formatDescription:format,sampleTiming:&timing,sampleBufferOut:&sample)==noErr,let sample else{throw SinkFailure(code:-5,message:"Amostra de vídeo indisponível")}
        let element=UnsafeMutableRawPointer(Unmanaged.passRetained(sample).toOpaque())
        if CMSimpleQueueEnqueue(queue,element:element) != noErr {Unmanaged<CMSampleBuffer>.fromOpaque(element).release();return 1}
        return 0
    }
}

@_cdecl("ponte_camera_sink_start")
public func ponteCameraSinkStart(_ width:UInt32,_ height:UInt32,_ fps:UInt32)->Int32 {
    ponteSinkLock.lock();defer{ponteSinkLock.unlock()}
    ponteSink?.stop();ponteSink=nil
    do{ponteSink=try PonteCameraSink(width:Int(width),height:Int(height),fps:Int32(fps));ponteSinkMessage="Câmera virtual do Mac pronta";return 0}
    catch let error as SinkFailure{ponteSinkMessage=error.message;return error.code}
    catch{ponteSinkMessage=error.localizedDescription;return -1}
}
@_cdecl("ponte_camera_sink_push_jpeg")
public func ponteCameraSinkPushJPEG(_ bytes:UnsafePointer<UInt8>?,_ length:UInt32)->Int32 {
    guard let bytes,length>0,length<=2*1024*1024 else{return -5}
    ponteSinkLock.lock();defer{ponteSinkLock.unlock()}
    guard let sink=ponteSink else{return -2}
    do{return try autoreleasepool{try sink.push(Data(bytes:bytes,count:Int(length)))}}
    catch let error as SinkFailure{ponteSinkMessage=error.message;return error.code}
    catch{ponteSinkMessage=error.localizedDescription;return -1}
}
@_cdecl("ponte_camera_sink_stop")
public func ponteCameraSinkStop(){ponteSinkLock.lock();defer{ponteSinkLock.unlock()};ponteSink?.stop();ponteSink=nil;ponteSinkMessage="Webcam remota desligada"}
@_cdecl("ponte_camera_sink_status_json")
public func ponteCameraSinkStatusJSON()->UnsafeMutablePointer<CChar>? {
    ponteSinkLock.lock();defer{ponteSinkLock.unlock()}
    var info:[String:Any]=["available":ponteVirtualDevice() != nil,"active":ponteSink != nil,"message":ponteSinkMessage,"component":"Ponte Desk Camera","device_uid":ponteDefaultCameraUID]
    if let sink=ponteSink{info.merge(sink.metadata){_,new in new}}
    guard let data=try? JSONSerialization.data(withJSONObject:info),let value=String(data:data,encoding:.utf8) else{return nil}
    return strdup(value)
}
