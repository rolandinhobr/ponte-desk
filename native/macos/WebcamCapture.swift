import AVFoundation
import ImageIO
import VideoToolbox
import UniformTypeIdentifiers

// Only external USB cameras are selectable; never default to FaceTime/Continuity.
func externalUSBCameras() -> [AVCaptureDevice] {
    AVCaptureDevice.DiscoverySession(deviceTypes:[.external], mediaType:.video, position:.unspecified).devices.filter { $0.transportType == 0x75736220 && $0.isConnected }
}
@_cdecl("ponte_camera_list_json")
public func ponteCameraListJSON()->UnsafeMutablePointer<CChar>? {
    let cameras=externalUSBCameras().map{["id":$0.uniqueID,"name":$0.localizedName,"external":true] as [String:Any]}
    guard let data=try? JSONSerialization.data(withJSONObject:["devices":cameras]),let json=String(data:data,encoding:.utf8) else{return nil}
    return strdup(json)
}
final class WebcamCapture:NSObject,AVCaptureVideoDataOutputSampleBufferDelegate {
    let session=AVCaptureSession()
    let queue=DispatchQueue(label:"br.com.ponte-desk.webcam.capture",qos:.userInitiated)
    let condition=NSCondition()
    var latest:Data?
    var ended=false
    var targetWidth=1920
    var targetHeight=1080
    var scalingContext:CGContext?
    func start(_ id:String,_ width:Int,_ height:Int,_ fps:Int)->Int32 {
        guard [(1280,720),(1920,1080)].contains(where:{$0.0==width && $0.1==height}),fps==30 else{return -4}
        guard AVCaptureDevice.authorizationStatus(for:.video) == .authorized else{return -2}
        guard let device=externalUSBCameras().first(where:{$0.uniqueID==id}) else{return -3}
        targetWidth=width;targetHeight=height
        let native=device.formats.first{format in
            let dimensions=CMVideoFormatDescriptionGetDimensions(format.formatDescription)
            return Int(dimensions.width)==width && Int(dimensions.height)==height && format.videoSupportedFrameRateRanges.contains{$0.minFrameRate<=Double(fps) && $0.maxFrameRate>=Double(fps)}
        }
        do {
            session.beginConfiguration();defer{session.commitConfiguration()}
            session.sessionPreset = width>=1920 ? .hd1920x1080 : .hd1280x720
            let input=try AVCaptureDeviceInput(device:device)
            guard session.canAddInput(input) else{return -4};session.addInput(input)
            let output=AVCaptureVideoDataOutput();output.alwaysDiscardsLateVideoFrames=true
            output.videoSettings=[kCVPixelBufferPixelFormatTypeKey as String:kCVPixelFormatType_32BGRA]
            output.setSampleBufferDelegate(self,queue:queue)
            guard session.canAddOutput(output) else{return -5};session.addOutput(output)
            try device.lockForConfiguration()
            if let native{device.activeFormat=native}
            if device.activeFormat.videoSupportedFrameRateRanges.contains(where:{$0.minFrameRate<=Double(fps) && $0.maxFrameRate>=Double(fps)}) {
                device.activeVideoMinFrameDuration=CMTime(value:1,timescale:CMTimeScale(fps));device.activeVideoMaxFrameDuration=CMTime(value:1,timescale:CMTimeScale(fps))
            }
            device.unlockForConfiguration()
        }catch{return -6}
        session.startRunning();return session.isRunning ? 0 : -7
    }
    func captureOutput(_ output:AVCaptureOutput,didOutput sampleBuffer:CMSampleBuffer,from connection:AVCaptureConnection) {
        autoreleasepool {
            guard let pixels=CMSampleBufferGetImageBuffer(sampleBuffer) else{return}
            var image:CGImage?
            guard VTCreateCGImageFromCVPixelBuffer(pixels,options:nil,imageOut:&image)==noErr,let image=image else{return}
            // A USB driver may ignore a session preset. Guarantee the declared
            // wire dimensions, and preserve aspect ratio for other webcams.
            let encodedImage:CGImage
            if image.width==targetWidth && image.height==targetHeight {encodedImage=image}
            else {
                if scalingContext==nil{scalingContext=CGContext(data:nil,width:targetWidth,height:targetHeight,bitsPerComponent:8,bytesPerRow:targetWidth*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGBitmapInfo.byteOrder32Little.rawValue|CGImageAlphaInfo.premultipliedFirst.rawValue)}
                guard let context=scalingContext else{return}
                context.setFillColor(CGColor(gray:0,alpha:1));context.fill(CGRect(x:0,y:0,width:targetWidth,height:targetHeight));context.interpolationQuality = .medium
                let scale=min(CGFloat(targetWidth)/CGFloat(image.width),CGFloat(targetHeight)/CGFloat(image.height))
                let fitted=CGSize(width:CGFloat(image.width)*scale,height:CGFloat(image.height)*scale)
                context.draw(image,in:CGRect(x:(CGFloat(targetWidth)-fitted.width)/2,y:(CGFloat(targetHeight)-fitted.height)/2,width:fitted.width,height:fitted.height))
                guard let resized=context.makeImage() else{return};encodedImage=resized
            }
            let bytes=NSMutableData()
            guard let encoder=CGImageDestinationCreateWithData(bytes,UTType.jpeg.identifier as CFString,1,nil) else{return}
            CGImageDestinationAddImage(encoder,encodedImage,[kCGImageDestinationLossyCompressionQuality:0.75] as CFDictionary)
            guard CGImageDestinationFinalize(encoder),bytes.length<=2*1024*1024 else{return}
            condition.lock();if !ended{latest=bytes as Data;condition.signal()};condition.unlock()
        }
    }
    func copy(_ buffer:UnsafeMutablePointer<UInt8>,_ capacity:Int)->Int32 {
        condition.lock();defer{condition.unlock()}
        let deadline=Date().addingTimeInterval(0.25)
        while latest==nil && !ended {if !condition.wait(until:deadline){return 0}}
        guard !ended else{return -1};guard let frame=latest else{return 0};latest=nil
        guard frame.count<=capacity else{return -2}
        frame.copyBytes(to:buffer,count:frame.count);return Int32(frame.count)
    }
    func stop(){condition.lock();ended=true;latest=nil;condition.broadcast();condition.unlock();session.stopRunning();queue.sync{};}
}
private var physicalCamera:WebcamCapture?
@_cdecl("ponte_camera_start")
public func ponteCameraStart(_ id:UnsafePointer<CChar>,_ width:UInt32,_ height:UInt32,_ fps:UInt32)->Int32 {
    guard physicalCamera==nil else{return -8}
    let camera=WebcamCapture();let result=camera.start(String(cString:id),Int(width),Int(height),Int(fps))
    if result==0{physicalCamera=camera};return result
}
@_cdecl("ponte_camera_copy_jpeg")
public func ponteCameraCopyJPEG(_ buffer:UnsafeMutablePointer<UInt8>,_ capacity:UInt32)->Int32 {physicalCamera?.copy(buffer,Int(capacity)) ?? -1}
@_cdecl("ponte_camera_stop")
public func ponteCameraStop(){physicalCamera?.stop();physicalCamera=nil}
