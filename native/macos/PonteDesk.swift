import AppKit
import ApplicationServices
import Darwin
import AVFoundation

// Native, lightweight interface for the integrated Ponte Desk service.
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    let root = ProcessInfo.processInfo.environment["PONTE_DESK_HOME"].map{URL(fileURLWithPath:$0)} ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/ponte-desk")
    var cameraProcess: Process?
    var cameraLocal: NSPopUpButton?
    var cameraRemote: NSPopUpButton?
    var cameraResolution: NSPopUpButton?
    var cameraState: NSTextField?
    var cameraDirection="idle"
    var cameraRefreshing=false
    var cameraStatusNotBefore=Date.distantPast.timeIntervalSince1970
    func cameraQualityArguments()->[String] {
        let quality=cameraResolution?.selectedItem?.representedObject as? String ?? "1080p"
        UserDefaults.standard.set(quality,forKey:"webcamQuality")
        return ["--quality",quality]
    }
    var cameraPeer:String? {
        guard let data=try? Data(contentsOf:root.appendingPathComponent("observer.json")),let settings=try? JSONSerialization.jsonObject(with:data) as? [String:Any] else{return nil}
        return (settings["peers"] as? [String])?.first
    }
    var observer: Process?
    let displays = DisplayController()
    var preview:Bool {ProcessInfo.processInfo.environment["PONTE_DESK_PREVIEW"] == "1"}
    var options:[String:Any] {
        var values:[String:Any] = ["local_name":"Mac","peer_name":"Windows","shared_monitor":false,"manage_displays":false,"protect_windows_fullscreen":true,"protect_mac_fullscreen":true,"webcam_enabled":true,"windows_monitor":"","mac_monitor_uuid":"","mac_input":0,"pc_input":0]
        for (key,value) in readJSON("preferences.json"){values[key]=value};return values
    }
    var localDisplayName:String {options["local_name"] as? String ?? "Mac"}
    var peerDisplayName:String {options["peer_name"] as? String ?? "Windows"}
    var peerPosition:String {
        guard let text=try? String(contentsOf:root.appendingPathComponent("config.toml"),encoding:.utf8),let regex=try? NSRegularExpression(pattern:#"(?m)^position\s*=\s*"(left|right|top|bottom)""#),let match=regex.firstMatch(in:text,range:NSRange(text.startIndex...,in:text)),let range=Range(match.range(at:1),in:text) else{return "left"}
        return String(text[range])
    }
    var permissionsButton:NSButton?;var cameraComponentButton:NSButton?;var inputSwitch:NSSwitch?;var inputState:NSTextField?;var cameraOrigin:NSSegmentedControl?
    var cameraStart:NSButton?;var cameraStop:NSButton?;var cameraNameHint:NSTextField?
    var monitorGroup:NSView?;var monitorStatus:NSTextField?;var autoDisplaySwitch:NSSwitch?;var layoutLabel:NSTextField?
    var settingsController:PonteSettingsController?
    var preparingCamera=false
    lazy var storage = PonteStorageController(owner:self)
    @objc func showSettings(){if settingsController?.window?.isVisible != true{settingsController=PonteSettingsController(owner:self)};settingsController?.showWindow(nil);NSApp.activate(ignoringOtherApps:true)}
    @objc func cameraOriginChanged(){
        let remote=cameraOrigin?.selectedSegment==1;cameraLocal?.isHidden=remote;cameraRemote?.isHidden = !remote
        cameraStart?.title=remote ? "Compartilhar neste Mac" : "Compartilhar em \(peerDisplayName)"
        UserDefaults.standard.set(remote ? 1:0,forKey:"webcamOrigin")
    }
    @objc func startSelectedWebcam(){guard options["webcam_enabled"] as? Bool != false else{return};if cameraOrigin?.selectedSegment==1{receiveWebcam()}else{sendWebcam()}}
    func availableMacMonitors()->[[String:Any]] {
        if preview{return [["id":"preview-display","name":"Monitor externo"]]}
        var values=NSScreen.screens.compactMap{screen->[String:Any]? in
            guard let id=(screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value,CGDisplayIsBuiltin(id)==0 else{return nil}
            return ["id":displays.uuid(id),"name":screen.localizedName]
        }
        let saved=readJSON("display-topology.json")
        if let target=saved["target"] as? UInt32,let screens=saved["screens"] as? [[String:Any]],let screen=screens.first(where:{$0["id"] as? UInt32==target}),let uuid=screen["uuid"] as? String,!values.contains(where:{$0["id"] as? String==uuid}) {values.append(["id":uuid,"name":"Monitor configurado (temporariamente desativado)"])}
        return values
    }
    func fetchMonitors(completion:@escaping([[String:Any]],String?)->Void){
        if preview{completion([["id":"example-display","name":"Monitor externo","input":17]],nil);return}
        guard let peer=cameraPeer else{completion([],"Configure o outro computador antes de identificar o monitor.");return}
        command(["settings","monitors","--peer",peer]){value in completion(value["monitors"] as? [[String:Any]] ?? [],value["error"] as? String)}
    }
    func saveOptions(_ values:[String:Any],position:String,completion:@escaping(String?)->Void){
        let old=options
        let nameValues=[values["local_name"] as? String ?? "",values["peer_name"] as? String ?? ""]
        guard nameValues.allSatisfy({!$0.isEmpty && $0.count<=64 && !$0.unicodeScalars.contains(where:CharacterSet.controlCharacters.contains)}) else{completion("Use nomes de até 64 caracteres, sem quebras de linha.");return}
        if values["shared_monitor"] as? Bool == true {
            let mac=values["mac_input"] as? Int ?? 0,pc=values["pc_input"] as? Int ?? 0
            guard mac>0,pc>0,mac<=255,pc<=255,mac != pc,!(values["mac_monitor_uuid"] as? String ?? "").isEmpty,!(values["windows_monitor"] as? String ?? "").isEmpty else{completion("Selecione o monitor nos dois computadores e identifique duas entradas diferentes.");return}
        }
        let persist:()->Void = {[weak self] in
            guard let self else{return}
            do {try FileManager.default.createDirectory(at:self.root,withIntermediateDirectories:true);let bytes=try JSONSerialization.data(withJSONObject:values,options:[.prettyPrinted,.sortedKeys]);try bytes.write(to:self.root.appendingPathComponent("preferences.json"),options:.atomic)}catch{completion(error.localizedDescription);return}
            if values["webcam_enabled"] as? Bool == false {self.stopWebcam()}
            if old["manage_displays"] as? Bool == true && (values["manage_displays"] as? Bool != true || values["shared_monitor"] as? Bool != true),!self.preview {self.displays.lastEnable=nil;self.displays.update(enable:true,root:self.root)}
            let visible=self.panel?.isVisible==true;self.panel?.close();self.panel=nil
            if visible{self.showPanel()};completion(nil)
        }
        guard position != peerPosition && !preview else{persist();return}
        guard let peer=cameraPeer else{completion("Configure um par de computadores antes de alterar a disposição.");return}
        let opposite=["left":"right","right":"left","top":"bottom","bottom":"top"][position] ?? "right"
        command(["settings","layout","--position",opposite,"--peer",peer]){[weak self] response in
            if let error=response["error"] as? String{completion(error);return}
            self?.command(["settings","layout","--position",position]){result in if let error=result["error"] as? String{completion(error)}else{persist()}}
        }
    }
    func command(_ arguments:[String],completion:@escaping([String:Any])->Void){
        if preview {completion([:]);return}
        let executable=Bundle.main.executableURL
        DispatchQueue.global(qos:.utility).async {
            let process=Process();process.executableURL=executable;process.arguments=arguments
            let pipe=Pipe();process.standardOutput=pipe;process.standardError=FileHandle.nullDevice
            var result:[String:Any]=[:]
            do{try process.run();let data=pipe.fileHandleForReading.readDataToEndOfFile();process.waitUntilExit();result=(try? JSONSerialization.jsonObject(with:data) as? [String:Any]) ?? [:];if process.terminationStatus != 0 && result["error"]==nil{result=["error":"Não foi possível comunicar com o outro computador."]}}catch{result=["error":error.localizedDescription]}
            DispatchQueue.main.async{completion(result)}
        }
    }

    func updateDisplay() {
        let state=readJSON("observer-status.json")
        guard let stamp=state["timestamp"] as? Double, Date().timeIntervalSince1970-stamp < 3,
              let display=state["display"] as? [String:Any], display["automatic"] as? Bool == true,
              display["peer_fresh"] as? Bool == true, let age=display["input_age_ms"] as? Double, age < 25000,
              let input=display["input"] as? UInt32, let mac=display["mac_input"] as? UInt32, let pc=display["pc_input"] as? UInt32 else { return }
        if input & 255 == mac { displays.update(enable:true,root:root) }
        else if input & 255 == pc { displays.update(enable:false,root:root) }
    }
    var item: NSStatusItem!
    var panel: NSWindow?
    var statusText: NSTextField?
    var detailText: NSTextField?
    var pauseButton: NSButton?
    var timer: Timer?
    var lastStart=Date.distantPast
    var terminationSource: DispatchSourceSignal?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        signal(SIGTERM, SIG_IGN)
        let source=DispatchSource.makeSignalSource(signal:SIGTERM,queue:.main)
        source.setEventHandler {
            // AppKit's asynchronous termination pumps a nested run loop.
            // Enter it outside a GCD main-queue handler so completion blocks
            // can still drain instead of deadlocking a SIGTERM update.
            RunLoop.main.perform(inModes:[.default,.modalPanel,.eventTracking]) {NSApp.terminate(nil)}
        }; source.resume(); terminationSource=source
        item = NSStatusBar.system.statusItem(withLength: 26)
        item.button?.image = PonteArtwork.menuIcon()
        item.button?.imagePosition = .imageOnly
        item.button?.setAccessibilityLabel("Ponte Desk")
        item.button?.toolTip = "Ponte Desk — seus computadores, juntos"
        let menu = NSMenu(); menu.delegate = self; item.menu = menu
        if !preview {startObserver()}
        storage.refreshAll()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            if let self=self, !self.preview, self.observer?.isRunning != true, Date().timeIntervalSince(self.lastStart)>10 {self.startObserver()}
            if self?.preview != true {self?.updateDisplay();self?.finishCameraSetupIfReady()}
            if self?.panel?.isVisible == true { self?.refreshPanel();self?.refreshCameraStatus() }
            self?.storage.tick()
        }
        if let timer {RunLoop.main.add(timer,forMode:.modalPanel)}
        if !CommandLine.arguments.contains("--background") { showPanel() }
    }

    func startObserver() {
        lastStart=Date()
        let process = Process()
        process.executableURL = Bundle.main.executableURL
        process.arguments = ["--capture-backend", "mac-os", "--emulation-backend", "mac-os", "daemon"]
        try? FileManager.default.createDirectory(at:root,withIntermediateDirectories:true)
        let log=root.appendingPathComponent("engine.log")
        if let size=(try? FileManager.default.attributesOfItem(atPath:log.path)[.size]) as? UInt64,size>1048576 {let previous=root.appendingPathComponent("engine.previous.log");try? FileManager.default.removeItem(at:previous);try? FileManager.default.moveItem(at:log,to:previous)}
        if !FileManager.default.fileExists(atPath:log.path){FileManager.default.createFile(atPath:log.path,contents:nil)}
        if let output=try? FileHandle(forWritingTo:log){_ = try? output.seekToEnd();process.standardOutput=output;process.standardError=output}
        do { try process.run(); observer = process }
        catch { NSLog("Unable to start Ponte Desk observer: %@", error.localizedDescription) }
    }

    func readJSON(_ name: String) -> [String: Any] {
        guard let data = try? Data(contentsOf: root.appendingPathComponent(name)),
              let dict = try? JSONSerialization.jsonObject(with: data) as? [String:Any] else { return [:] }
        return dict
    }

    func mode()->String {
        if preview{return "Prévia da interface"}
        if !AXIsProcessTrusted() || !CGPreflightListenEventAccess(){return "Permissão necessária"}
        if paused{return "Teclado e mouse em pausa"}
        let state=readJSON("observer-status.json")
        guard let stamp=state["timestamp"] as? Double,Date().timeIntervalSince1970-stamp<8,let age=state["peer_age"] as? Double,age<6 else{return "Aguardando o outro computador"}
        if let policy=state["policy"] as? [Any],let reason=policy.last as? String,reason.contains("agente"){return "Controle temporário isolado"}
        let engine=readJSON("engine-status.json")
        if engine["capture_installed"] as? Bool == true || engine["emulation_enabled"] as? Bool == true{return "Pronto para compartilhar"}
        return "Controle local protegido"
    }

    var paused: Bool { FileManager.default.fileExists(atPath: root.appendingPathComponent("manual-pause").path) }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let state = NSMenuItem(title: mode(), action: nil, keyEquivalent: ""); state.isEnabled = false; menu.addItem(state)
        menu.addItem(.separator())
        let show = NSMenuItem(title: "Abrir Ponte Desk", action: #selector(showPanel), keyEquivalent: ""); show.target = self; menu.addItem(show)
        let toggle = NSMenuItem(title: paused ? "Retomar teclado e mouse" : "Pausar teclado e mouse", action: #selector(togglePause), keyEquivalent: ""); toggle.target = self; menu.addItem(toggle)
        let settings=NSMenuItem(title:"Ajustes…",action:#selector(showSettings),keyEquivalent:",");settings.target=self;menu.addItem(settings)
        let cameraStop=NSMenuItem(title:"Parar compartilhamento da webcam",action:#selector(stopWebcam),keyEquivalent:"");cameraStop.target=self;menu.addItem(cameraStop)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Encerrar Ponte Desk", action: #selector(quit), keyEquivalent: "q"); quit.target = self; menu.addItem(quit)
    }

    @objc func showPanel(){
        if panel==nil{panel=buildPontePanel(self);cameraOrigin?.selectedSegment=UserDefaults.standard.integer(forKey:"webcamOrigin");cameraOriginChanged();refreshCameras()}
        refreshPanel();panel?.makeKeyAndOrderFront(nil);NSApp.activate(ignoringOtherApps:true)
        storage.refreshAll()
    }
    func refreshPanel(){
        statusText?.stringValue=mode();item?.button?.image=PonteArtwork.menuIcon(paused:paused)
        inputSwitch?.state=paused ? .off:.on
        let state=readJSON("observer-status.json"),policy=state["policy"] as? [Any]
        inputState?.stringValue=preview ? "Disponível nos dois sentidos" : policy?.last as? String ?? "Configure o par para começar"
        let permission=preview || (AXIsProcessTrusted() && CGPreflightListenEventAccess())
        detailText?.stringValue=permission ? "":"Autorize controle e monitoração de entrada nas permissões deste Mac."
        detailText?.isHidden=permission;permissionsButton?.isHidden=permission
        let directions=["left":"à esquerda","right":"à direita","top":"acima","bottom":"abaixo"]
        layoutLabel?.stringValue="\(peerDisplayName) \(directions[peerPosition] ?? "ao lado")";layoutLabel?.isHidden=false
        let shared=options["shared_monitor"] as? Bool == true
        monitorGroup?.isHidden = !shared;autoDisplaySwitch?.state=options["manage_displays"] as? Bool == true ? .on:.off
        monitorStatus?.stringValue=policy?.last as? String ?? "Aguardando monitor"
        let cameraEnabled=options["webcam_enabled"] as? Bool != false
        cameraStart?.isEnabled=cameraEnabled && !preview
        cameraStop?.isEnabled=cameraDirection != "idle"
        let info=cameraComponentStatus();let ready=info["ready"] as? Bool == true;cameraNameHint?.isHidden = !ready;cameraComponentButton?.isHidden=ready
        if !cameraEnabled{cameraState?.stringValue="Webcam desativada nos ajustes"}
    }

    @objc func togglePause() {
        let path=root.appendingPathComponent("manual-pause")
        if paused { try? FileManager.default.removeItem(at:path) }
        else { try? Data("paused".utf8).write(to:path,options:.atomic) }
        refreshPanel()
    }

    @objc func openPermissions() {
        guard !preview else{return}
        let options=[kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String:true] as CFDictionary
        _ = AXIsProcessTrustedWithOptions(options)
        if let url=URL(string:"x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") { NSWorkspace.shared.open(url) }
    }

    func cameraCommand(_ arguments:[String],completion:@escaping([String:Any])->Void){command(["webcam"]+arguments,completion:completion)}
    @objc func refreshCameras() {
        if preview {cameraLocal?.removeAllItems();cameraLocal?.addItem(withTitle:"Câmera USB externa");cameraRemote?.removeAllItems();cameraRemote?.addItem(withTitle:"Câmera USB externa");return}
        let local=externalUSBCameras();let selected=cameraLocal?.selectedItem?.representedObject as? String
        cameraLocal?.removeAllItems()
        if local.isEmpty {cameraLocal?.addItem(withTitle:"Nenhuma webcam externa neste Mac")}
        for device in local {cameraLocal?.addItem(withTitle:device.localizedName);cameraLocal?.lastItem?.representedObject=device.uniqueID;if device.uniqueID==selected {cameraLocal?.select(cameraLocal!.lastItem)}}
        guard !cameraRefreshing,let peer=cameraPeer else{return};cameraRefreshing=true
        cameraCommand(["devices","--peer",peer]){[weak self] result in
            guard let self=self else{return};self.cameraRefreshing=false;let selected=self.cameraRemote?.selectedItem?.representedObject as? String;self.cameraRemote?.removeAllItems()
            let devices=result["devices"] as? [[String:Any]] ?? []
            if devices.isEmpty {self.cameraRemote?.addItem(withTitle:"Nenhuma webcam externa no outro computador")}
            for device in devices {guard let id=device["id"] as? String,let name=device["name"] as? String else{continue};self.cameraRemote?.addItem(withTitle:name);self.cameraRemote?.lastItem?.representedObject=id;if id==selected {self.cameraRemote?.select(self.cameraRemote!.lastItem)}}
            if let error=result["error"] as? String {self.cameraState?.stringValue=error}
        }
    }
    func refreshCameraStatus() {
        let state=readJSON("webcam-status.json")
        if cameraDirection=="stopping"{return}
        if let stamp=state["timestamp_ms"] as? Double,stamp/1000>=cameraStatusNotBefore,Date().timeIntervalSince1970-stamp/1000<4,let mode=state["mode"] as? String,mode=="sending" || mode=="receiving" || mode=="error" {
            cameraDirection=mode=="error" ? "idle":mode
            cameraStop?.isEnabled=mode != "error";cameraState?.stringValue=state["message"] as? String ?? mode
            return
        }
        if cameraDirection=="idle"{cameraState?.stringValue="Webcam desligada. Somente a câmera externa selecionada será compartilhada."}
        else if cameraDirection=="sending",cameraProcess?.isRunning != true {cameraDirection="idle";cameraState?.stringValue="O envio da webcam terminou."}
    }
    @objc func sendWebcam() {
        guard !preview,options["webcam_enabled"] as? Bool != false else{return}
        guard let device=cameraLocal?.selectedItem?.representedObject as? String,let peer=cameraPeer else{cameraState?.stringValue="Selecione uma webcam USB externa conectada ao Mac.";return}
        let qualityArguments=cameraQualityArguments()
        let begin={ [weak self] in
            guard let self=self else{return}
            self.stopWebcam()
            self.cameraState?.stringValue="Preparando webcam no Windows…"
            let process=Process();process.executableURL=Bundle.main.executableURL;process.arguments=["webcam","send","--device",device,"--peer",peer,"--parent",String(ProcessInfo.processInfo.processIdentifier)]+qualityArguments;process.standardOutput=FileHandle.nullDevice;process.standardError=FileHandle.nullDevice
            do {try process.run();self.cameraProcess=process;self.cameraDirection="sending"}catch{self.cameraState?.stringValue=error.localizedDescription}
        }
        switch AVCaptureDevice.authorizationStatus(for:.video) {
        case .authorized:begin()
        case .notDetermined:AVCaptureDevice.requestAccess(for:.video){granted in DispatchQueue.main.async{if granted{begin()}else{self.cameraState?.stringValue="Autorize Câmera para o Ponte Desk em Ajustes → Privacidade e Segurança."}}}
        default:cameraState?.stringValue="Autorize Câmera para o Ponte Desk em Ajustes → Privacidade e Segurança."
        }
    }
    @objc func receiveWebcam() {
        guard !preview,options["webcam_enabled"] as? Bool != false else{return}
        if let bytes=ponteCameraSinkStatusJSON(){let json=String(cString:bytes);free(bytes);if let data=json.data(using:.utf8),let state=try? JSONSerialization.jsonObject(with:data) as? [String:Any],state["available"] as? Bool != true {cameraState?.stringValue="Instale e autorize primeiro o componente de câmera do Mac.";return}}
        guard let device=cameraRemote?.selectedItem?.representedObject as? String,let peer=cameraPeer else{cameraState?.stringValue="Selecione uma webcam USB externa do Windows (Atualizar câmeras).";return}
        if let cameraProcess=cameraProcess,cameraProcess.isRunning{cameraProcess.interrupt()};cameraProcess=nil;cameraDirection="receiving";cameraState?.stringValue="Preparando webcam do Windows…"
        cameraCommand(["request","--peer",peer,"--device",device]+cameraQualityArguments()){[weak self] result in
            if let error=result["error"] as? String {self?.cameraState?.stringValue=error;self?.cameraDirection="idle"}
            else{self?.cameraState?.stringValue="Aguardando imagem do Windows; selecione Ponte Desk Camera no aplicativo."
                DispatchQueue.main.asyncAfter(deadline:.now()+3){[weak self] in guard let self=self,self.cameraDirection=="receiving" else{return};self.cameraCommand(["peer-status","--peer",peer]){[weak self] status in if status["mode"] as? String == "error" {self?.cameraState?.stringValue=status["message"] as? String ?? "A webcam do Windows não iniciou.";self?.cameraDirection="idle"}}}
            }
        }
    }
    @objc func stopWebcam(){
        if let cameraProcess,cameraProcess.isRunning{cameraProcess.interrupt()};cameraProcess=nil
        cameraDirection="stopping";cameraStop?.isEnabled=false;cameraStatusNotBefore=Date().timeIntervalSince1970
        cameraState?.stringValue="Encerrando o compartilhamento…"
        guard let peer=cameraPeer else{cameraDirection="idle";cameraState?.stringValue="Webcam desligada";return}
        cameraCommand(["stop","--peer",peer]){[weak self] result in
            guard let self else{return}
            self.cameraDirection="idle"
            self.cameraState?.stringValue=result["ok"] as? Bool == true ? "Webcam desligada" : "Não foi possível confirmar a parada no outro computador."
        }
    }
    @objc func installCameraDriver(){beginCameraSetup()}
    @objc func toggleDisplays(){var values=options;values["manage_displays"]=autoDisplaySwitch?.state == .on;saveOptions(values,position:peerPosition){[weak self] error in if let error{self?.monitorStatus?.stringValue=error}}}
    @objc func restoreDisplays(){guard !preview else{return};var values=options;values["manage_displays"]=false;saveOptions(values,position:peerPosition){_ in};displays.lastEnable=nil;displays.update(enable:true,root:root)}
    @objc func quit() { NSApp.terminate(nil) }
    func applicationShouldTerminate(_ sender:NSApplication)->NSApplication.TerminateReply {
        if preview{return .terminateNow}
        storage.prepareToQuit { [weak self] ready in
            DispatchQueue.main.async {
                if !ready {self?.showPanel()}
                sender.reply(toApplicationShouldTerminate:ready)
            }
        }
        return .terminateLater
    }
    func applicationWillTerminate(_ notification: Notification) {
        timer?.invalidate()
        storage.invalidate()
        if let cameraProcess=cameraProcess,cameraProcess.isRunning{cameraProcess.interrupt()}
        if let observer=observer, observer.isRunning { kill(observer.processIdentifier, SIGINT) }
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender:NSApplication)->Bool { false }
    func applicationShouldHandleReopen(_ sender:NSApplication,hasVisibleWindows flag:Bool)->Bool { showPanel(); return true }
}

@_cdecl("ponte_desk_app_main")
public func ponteDeskAppMain() {
    let app=NSApplication.shared
    let delegate=AppDelegate()
    app.delegate=delegate
    withExtendedLifetime(delegate) { app.run() }
}

enum MonitorIdentity {
    static var configuration:[String:Any]{
        let root=ProcessInfo.processInfo.environment["PONTE_DESK_HOME"].map{URL(fileURLWithPath:$0)} ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/ponte-desk")
        return (try? Data(contentsOf:root.appendingPathComponent("preferences.json"))).flatMap{try? JSONSerialization.jsonObject(with:$0) as? [String:Any]} ?? [:]
    }
    static func matches(_ id:UInt32)->Bool {
        guard let expected=configuration["mac_monitor_uuid"] as? String,!expected.isEmpty,let uuid=CGDisplayCreateUUIDFromDisplayID(id)?.takeRetainedValue() else{return false}
        return (CFUUIDCreateString(nil,uuid) as String).caseInsensitiveCompare(expected) == .orderedSame
    }
}
// Called by the headless core. Metadata only: no screenshots or input hooks.
@_cdecl("ponte_platform_probe")
public func pontePlatformProbe() -> UInt32 {
    autoreleasepool {
        var ids=[CGDirectDisplayID](repeating:0,count:32);var count:UInt32=0
        guard CGGetActiveDisplayList(32,&ids,&count) == .success else {return 0}
        let active=Array(ids.prefix(Int(count)))
        var result:UInt32=active.contains(where:{MonitorIdentity.matches($0)}) ? 1 : 0
        if !active.isEmpty && active.allSatisfy({CGDisplayIsAsleep($0) != 0}) {result |= 4}
        if let windows=CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]],
           let front=windows.first(where:{($0[kCGWindowLayer as String] as? Int)==0 && ($0[kCGWindowAlpha as String] as? Double ?? 1)>0}),
           let bounds=front[kCGWindowBounds as String] as? [String:Any], let rect=CGRect(dictionaryRepresentation:bounds as CFDictionary),
           active.contains(where:{let b=CGDisplayBounds($0);return abs(b.minX-rect.minX)<2 && abs(b.minY-rect.minY)<2 && abs(b.width-rect.width)<2 && abs(b.height-rect.height)<2}) {result |= 2}
        return result
    }
}

struct SavedScreen: Codable {
    var id:UInt32;var uuid:String;var vendor:UInt32;var model:UInt32;var serial:UInt32
    var x:Int32;var y:Int32;var width:Int;var height:Int;var pixelWidth:Int;var pixelHeight:Int;var refresh:Double
}
struct SavedTopology: Codable {var target:UInt32;var disabled:Bool;var screens:[SavedScreen]}
final class DisplayController {
    typealias EnableFn = @convention(c) (CGDisplayConfigRef,UInt32,Bool)->Int32
    let library=dlopen("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics",RTLD_LAZY)
    lazy var enable:EnableFn? = {guard let library=self.library,let symbol=dlsym(library,"CGSConfigureDisplayEnabled") else{return nil};return unsafeBitCast(symbol,to:EnableFn.self)}()
    var lastAttempt=Date.distantPast
    var lastEnable:Bool?
    func list()->[UInt32] {var ids=[UInt32](repeating:0,count:32);var n:UInt32=0;guard CGGetActiveDisplayList(32,&ids,&n) == .success else{return []};return Array(ids.prefix(Int(n)))}
    func uuid(_ id:UInt32)->String {guard let ref=CGDisplayCreateUUIDFromDisplayID(id)?.takeRetainedValue() else{return ""};return CFUUIDCreateString(nil,ref) as String}
    func snapshot(_ ids:[UInt32],target:UInt32)->SavedTopology? {
        var screens:[SavedScreen]=[]
        for id in ids {
            guard let mode=CGDisplayCopyDisplayMode(id) else{return nil}
            let b=CGDisplayBounds(id)
            screens.append(SavedScreen(id:id,uuid:uuid(id),vendor:CGDisplayVendorNumber(id),model:CGDisplayModelNumber(id),serial:CGDisplaySerialNumber(id),x:Int32(b.minX),y:Int32(b.minY),width:mode.width,height:mode.height,pixelWidth:mode.pixelWidth,pixelHeight:mode.pixelHeight,refresh:mode.refreshRate))
        }
        return SavedTopology(target:target,disabled:false,screens:screens)
    }
    func save(_ value:SavedTopology,_ path:URL) {if let data=try? JSONEncoder().encode(value){try? data.write(to:path,options:.atomic)}}
    func report(_ text:String,_ root:URL) {let value:[String:Any]=["timestamp":Date().timeIntervalSince1970,"message":text];if let data=try? JSONSerialization.data(withJSONObject:value){try? data.write(to:root.appendingPathComponent("display-status.json"),options:.atomic)}}
    func update(enable desired:Bool,root:URL) {
        precondition(Thread.isMainThread)
        let ids=list();let path=root.appendingPathComponent("display-topology.json")
        var stored=(try? Data(contentsOf:path)).flatMap{try? JSONDecoder().decode(SavedTopology.self,from:$0)}
        let matches=ids.filter{MonitorIdentity.matches($0)}
        guard matches.count<=1 else{report("Seleção de monitor ambígua; revise os ajustes",root);return}
        let target=matches.first
        if desired {
            if let target=target {
                // Preserve the user's latest arrangement while both screens are in use.
                if stored?.disabled != true, let value=snapshot(ids,target:target) {if value.screens != stored?.screens {save(value,path)}}
                if stored?.disabled != true {return}
            }
        } else if target == nil {return}
        if lastEnable == desired && Date().timeIntervalSince(lastAttempt)<10 {return}
        lastEnable=desired;lastAttempt=Date()
        guard let configure=enable else {report("API de desconexão indisponível neste macOS",root);return}
        if !desired {
            // Never remove the last usable display, even if the lid was closed.
            guard ids.contains(where:{CGDisplayIsBuiltin($0) != 0 && CGDisplayIsAsleep($0)==0}),let target=target,
                  var value=snapshot(ids,target:target) else {report("Aguardando tela integrada ativa",root);return}
            value.disabled=true;save(value,path) // recover after a crash
            stored=value
        }
        guard var value=stored else {return}
        let id=target ?? value.target
        if desired,let original=value.screens.first(where:{$0.id==value.target}),CGDisplayVendorNumber(id) != 0,
           (CGDisplayVendorNumber(id) != original.vendor || CGDisplaySerialNumber(id) != original.serial) {report("Identidade do monitor mudou; restauração suspensa",root);return}
        var reference:CGDisplayConfigRef?
        guard CGBeginDisplayConfiguration(&reference) == .success,let configuration=reference else{return}
        guard configure(configuration,id,desired)==0 else{CGCancelDisplayConfiguration(configuration);report("macOS recusou a troca de tela",root);return}
        guard CGCompleteDisplayConfiguration(configuration,.forAppOnly) == .success else{report("Falha ao concluir troca de tela",root);return}
        if desired {
            guard CGDisplayIsActive(id) != 0 else {report("Aguardando o monitor ser reativado pelo macOS",root);return}
            // Enable first, then restore matching modes/origins in one transaction.
            var restore:CGDisplayConfigRef?
            if CGBeginDisplayConfiguration(&restore) == .success,let transaction=restore {
                var valid=true
                for screen in value.screens {
                    let current=list().first(where:{uuid($0)==screen.uuid}) ?? screen.id
                    guard CGDisplayIsActive(current) != 0 else{continue}
                    if let modes=CGDisplayCopyAllDisplayModes(current,nil) as? [CGDisplayMode],let mode=modes.first(where:{$0.width==screen.width && $0.height==screen.height && $0.pixelWidth==screen.pixelWidth && $0.pixelHeight==screen.pixelHeight && abs($0.refreshRate-screen.refresh)<0.5}) {
                        if CGConfigureDisplayWithDisplayMode(transaction,current,mode,nil) != .success {valid=false}
                    }
                    if CGConfigureDisplayOrigin(transaction,current,screen.x,screen.y) != .success {valid=false}
                }
                if valid {_ = CGCompleteDisplayConfiguration(transaction,.forAppOnly)} else {CGCancelDisplayConfiguration(transaction)}
            }
            value.disabled=false;save(value,path)
        }
        report(desired ? "Monitor reativado; disposição restaurada" : "Monitor em uso pelo outro computador; tela integrada ativa",root)
    }
}
extension SavedScreen: Equatable {}
