// CMIO component integration. Signed provider attribution is in NOTICE.
import AppKit
import CoreMediaIO
import ImageIO
import Security

private let componentUID="5C0B4E2A-1F3D-4A6B-9C7E-1D2E3F4A5B6C"
private let componentName="Ponte Desk Camera"
private func componentAddress(_ selector:UInt32)->CMIOObjectPropertyAddress{CMIOObjectPropertyAddress(mSelector:selector,mScope:CMIOObjectPropertyScope(kCMIOObjectPropertyScopeGlobal),mElement:CMIOObjectPropertyElement(kCMIOObjectPropertyElementMain))}
private func componentString(_ object:CMIOObjectID,_ selector:UInt32)->String? {
    var address=componentAddress(selector);var value:Unmanaged<CFString>?;var used:UInt32=0
    guard CMIOObjectGetPropertyData(object,&address,0,nil,UInt32(MemoryLayout<Unmanaged<CFString>?>.size),&used,&value)==noErr else{return nil}
    return value?.takeRetainedValue() as String?
}
private func componentDevice()->CMIOObjectID? {
    var address=componentAddress(UInt32(kCMIOHardwarePropertyDevices));var count:UInt32=0
    guard CMIOObjectGetPropertyDataSize(CMIOObjectID(kCMIOObjectSystemObject),&address,0,nil,&count)==noErr,count>0,count<=4096 else{return nil}
    var devices=[CMIOObjectID](repeating:0,count:Int(count)/4);var used:UInt32=0
    guard CMIOObjectGetPropertyData(CMIOObjectID(kCMIOObjectSystemObject),&address,0,nil,count,&used,&devices)==noErr else{return nil}
    return devices.prefix(Int(used)/4).first{componentString($0,UInt32(kCMIODevicePropertyDeviceUID))?.caseInsensitiveCompare(componentUID) == .orderedSame}
}
private func componentSetString(_ object:CMIOObjectID,_ selector:UInt32,_ text:String)->OSStatus {
    var address=componentAddress(selector);let value=text as CFString;var pointer=Unmanaged.passUnretained(value).toOpaque()
    return withExtendedLifetime(value){CMIOObjectSetPropertyData(object,&address,0,nil,UInt32(MemoryLayout<UnsafeMutableRawPointer>.size),&pointer)}
}
func cameraComponentStatus()->[String:Any] {
    guard let device=componentDevice() else{return ["available":false,"ready":false]}
    let name=componentString(device,UInt32(kCMIOObjectPropertyName)) ?? ""
    return ["available":true,"ready":name==componentName,"name":name]
}
private func cameraStandbyJPEG()->Data? {
    let width=1920,height=1080
    guard let ctx=CGContext(data:nil,width:width,height:height,bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else{return nil}
    ctx.setFillColor(CGColor(red:0.055,green:0.065,blue:0.105,alpha:1));ctx.fill(CGRect(x:0,y:0,width:width,height:height))
    ctx.setStrokeColor(CGColor(red:0.57,green:0.60,blue:0.95,alpha:1));ctx.setLineWidth(12);ctx.setLineCap(.round);ctx.setLineJoin(.round)
    for x in [CGFloat(825),CGFloat(995)] {let path=CGPath(roundedRect:CGRect(x:x,y:470,width:100,height:140),cornerWidth:18,cornerHeight:18,transform:nil);ctx.addPath(path);ctx.strokePath()}
    ctx.move(to:CGPoint(x:905,y:540));ctx.addLine(to:CGPoint(x:1015,y:540));ctx.strokePath()
    guard let image=ctx.makeImage() else{return nil};let data=NSMutableData()
    guard let dest=CGImageDestinationCreateWithData(data,"public.jpeg" as CFString,1,nil) else{return nil}
    CGImageDestinationAddImage(dest,image,[kCGImageDestinationLossyCompressionQuality:0.85] as CFDictionary)
    guard CGImageDestinationFinalize(dest) else{return nil};return data as Data
}
func prepareCameraComponent()->[String:Any] {
    guard let device=componentDevice() else{return ["ready":false,"message":"Ative o componente de câmera nas permissões do macOS."]}
    let state=cameraComponentStatus()
    if state["ready"] as? Bool != true {
        let status=componentSetString(device,0x6c6c636e,componentName)
        return ["ready":false,"renaming":status==noErr,"message":status==noErr ? "Feche os aplicativos que usam essa câmera para concluir a identificação." : "Não foi possível identificar a câmera virtual."]
    }
    guard let data=cameraStandbyJPEG(),componentSetString(device,0x706c6368,data.base64EncodedString())==noErr else{return ["ready":false,"message":"Não foi possível preparar a imagem de espera da câmera."]}
    return ["available":true,"ready":true,"name":componentName]
}
@_cdecl("ponte_camera_component_prepare_json")
public func ponteCameraComponentPrepareJSON()->UnsafeMutablePointer<CChar>? {
    guard let data=try? JSONSerialization.data(withJSONObject:prepareCameraComponent()),let text=String(data:data,encoding:.utf8) else{return nil};return strdup(text)
}
private func verifiedCameraInstaller(_ url:URL)->Bool {
    var code:SecStaticCode?;var requirement:SecRequirement?
    guard SecStaticCodeCreateWithPath(url as CFURL,[],&code)==errSecSuccess,let code,
          SecRequirementCreateWithString("anchor apple generic and certificate leaf[subject.OU] = \"2TWQF4T93E\" and identifier \"com.adrbn.LiveLoop\"" as CFString,[],&requirement)==errSecSuccess,let requirement else{return false}
    return SecStaticCodeCheckValidity(code,SecCSFlags(rawValue:kSecCSCheckAllArchitectures|kSecCSStrictValidate),requirement)==errSecSuccess
}
extension AppDelegate {
    var componentInstallerURL:URL {URL(fileURLWithPath:"/Applications/Ponte Desk Camera Component.app")}
    @objc func beginCameraSetup(){
        guard !preview else{return}
        let status=cameraComponentStatus()
        if status["ready"] as? Bool == true {cameraState?.stringValue="Ponte Desk Camera está pronta para uso.";return}
        if status["available"] as? Bool == true && !FileManager.default.fileExists(atPath:root.appendingPathComponent("camera-component-owned").path) {
            let alert=NSAlert();alert.messageText="Usar o componente de câmera existente?";alert.informativeText="Esse componente já está instalado e será compartilhado com o Ponte Desk. Seu nome na lista de câmeras e sua imagem de espera serão personalizados. Isso pode afetar outro aplicativo que já utilize o mesmo componente.";alert.addButton(withTitle:"Usar e personalizar");alert.addButton(withTitle:"Cancelar");guard alert.runModal() == .alertFirstButtonReturn else{return}
        }
        if status["available"] as? Bool != true {
            let helper=Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/CameraComponent.app")
            guard FileManager.default.fileExists(atPath:helper.path) else{cameraState?.stringValue="O componente de câmera não foi incluído neste pacote.";return}
            do {if !FileManager.default.fileExists(atPath:componentInstallerURL.path){try FileManager.default.copyItem(at:helper,to:componentInstallerURL)}}catch{cameraState?.stringValue=error.localizedDescription;return}
            guard verifiedCameraInstaller(componentInstallerURL) else{cameraState?.stringValue="O componente de câmera não passou na verificação. Reinstale o pacote oficial.";return}
            let alert=NSAlert();alert.messageText="Ativar a câmera do Ponte Desk";alert.informativeText="O componente incluído abrirá um assistente. Escolha Install Camera e aprove a extensão em Ajustes → Geral → Itens de Início de Sessão e Extensões → Extensões de Câmera. Não é necessário autorizar acesso à câmera física para esse instalador. Depois da ativação, o Ponte Desk encerra o instalador e prepara o nome da câmera.";alert.addButton(withTitle:"Abrir assistente");alert.addButton(withTitle:"Cancelar");guard alert.runModal() == .alertFirstButtonReturn else{return}
            NSWorkspace.shared.open(componentInstallerURL)
        }
        try? Data("Ponte Desk camera component".utf8).write(to:root.appendingPathComponent("camera-component-owned"),options:.atomic)
        preparingCamera=true;finishCameraSetupIfReady()
    }
    func finishCameraSetupIfReady(){
        guard preparingCamera,cameraComponentStatus()["available"] as? Bool == true else{return}
        let hosts=NSRunningApplication.runningApplications(withBundleIdentifier:"com.adrbn.LiveLoop").filter{$0.bundleURL?.standardizedFileURL==componentInstallerURL.standardizedFileURL}
        if !hosts.isEmpty {for host in hosts{host.terminate()};return}
        let result=prepareCameraComponent();cameraState?.stringValue=result["message"] as? String ?? "Ponte Desk Camera está pronta para uso."
        if result["ready"] as? Bool == true {preparingCamera=false;refreshPanel()}
    }
}
