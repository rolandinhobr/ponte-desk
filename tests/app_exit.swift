// Full production delegate exit regression. No hooks, cameras, SMB or UI input.
import AppKit
import Darwin

private func check(_ condition: @autoclosure ()->Bool,_ message:String){
    if !condition(){fputs("FAIL: \(message)\n",stderr);exit(1)}
}
private final class ExitFixture:NSObject,NSApplicationDelegate {
    let owner=AppDelegate()
    let root:URL
    var timer:Timer?
    var watchdog:DispatchWorkItem?
    var began:Date?
    let original=Data("{\"active\":true,\"phase\":\"error\",\"cleanup_confirmed\":false,\"error\":\"offline fixture\"}".utf8)
    init(root:URL){self.root=root;super.init()}
    func sleeper()->Process {
        let p=Process();p.executableURL=URL(fileURLWithPath:"/usr/bin/python3")
        p.arguments=["-c","import time; time.sleep(30)"]
        p.standardOutput=FileHandle.nullDevice;p.standardError=FileHandle.nullDevice
        try! p.run();return p
    }
    func applicationDidFinishLaunching(_ notification:Notification){
        try! original.write(to:root.appendingPathComponent("storage-host.json"))
        owner.observer=sleeper();owner.cameraProcess=sleeper()
        owner.command(["--blocked-cli"]){_ in}
        let deadline=DispatchWorkItem{fputs("FAIL: explicit quit was blocked\n",stderr);exit(1)}
        watchdog=deadline;DispatchQueue.global().asyncAfter(deadline:.now()+4,execute:deadline)
        timer=Timer.scheduledTimer(withTimeInterval:0.02,repeats:true){[weak self] _ in
            guard let self,FileManager.default.fileExists(atPath:self.root.appendingPathComponent("worker.pid").path) else{return}
            self.timer?.invalidate()
            let window=NSWindow(contentRect:NSRect(x:-20000,y:-20000,width:100,height:100),styleMask:[.titled,.closable],backing:.buffered,defer:false)
            self.owner.panel=window;window.delegate=self.owner
            self.began=Date()
            // The normal close-button route, without showing or clicking a
            // desktop window. It must reach terminateNow despite pending work.
            window.performClose(nil)
        }
    }
    func applicationShouldTerminate(_ sender:NSApplication)->NSApplication.TerminateReply {
        owner.applicationShouldTerminate(sender)
    }
    func applicationWillTerminate(_ notification:Notification){
        owner.applicationWillTerminate(notification)
        check(began != nil && Date().timeIntervalSince(began!)<1.3,"Exit exceeded its local deadline")
        check((try? Data(contentsOf:root.appendingPathComponent("storage-host.json")))==original,"Exit falsely marked unknown cleanup successful")
        let worker=Int32((try! String(contentsOf:root.appendingPathComponent("worker.pid"),encoding:.utf8)).trimmingCharacters(in:.whitespacesAndNewlines))!
        let until=Date().addingTimeInterval(0.3)
        while kill(worker,0)==0 && Date()<until{Thread.sleep(forTimeInterval:0.01)}
        check(kill(worker,0) != 0,"Blocked CLI survived exit")
        check(owner.observer?.isRunning != true,"Owned engine survived exit")
        watchdog?.cancel();timer?.invalidate()
        try? FileManager.default.removeItem(at:root)
        print("PASS: window close always exits; blocked command cancelled; unknown SD state preserved")
        fflush(stdout)
    }
}
@main struct TestMain {
    static func main(){
        if CommandLine.arguments.contains("--blocked-cli"){
            let root=ProcessInfo.processInfo.environment["PONTE_DESK_HOME"]!
            try! Data("\(getpid())".utf8).write(to:URL(fileURLWithPath:root).appendingPathComponent("worker.pid"))
            Thread.sleep(forTimeInterval:30);return
        }
        let root=FileManager.default.temporaryDirectory.appendingPathComponent("ponte-exit-test-"+UUID().uuidString)
        try! FileManager.default.createDirectory(at:root,withIntermediateDirectories:true)
        setenv("PONTE_DESK_HOME",root.path,1)
        let app=NSApplication.shared;app.setActivationPolicy(.prohibited)
        let delegate=ExitFixture(root:root);app.delegate=delegate
        withExtendedLifetime(delegate){app.run()}
    }
}
