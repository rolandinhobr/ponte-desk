// Behavioural tests for PonteStorageController. No real CLI, volumes, helpers,
// network, permissions, windows or file sharing are touched by this executable.
// Build with PonteStorage.swift, AppKit and ServiceManagement (not PonteDesk.swift).
import AppKit
import Darwin

private func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else {
        fputs("FAIL: \(message)\n", stderr)
        fflush(stderr)
        exit(1)
    }
}

private func descendants(_ view: NSView) -> [NSView] {
    [view] + view.subviews.flatMap(descendants)
}

/// Satisfies the production controller's owner contract with an in-memory CLI.
/// Also serves as the delegate of the final, windowless NSApplication fixture.
final class AppDelegate: NSObject, NSApplicationDelegate {
    let preview = false
    var cameraPeer: String? = "192.0.2.20"
    let sessionID = String(repeating: "a", count: 32)
    var calls: [[String]] = []
    var failUnmount = false
    var malformedPublish = false
    var asyncCallbacks = false
    var holdCallbacks = false
    var deferredCallbacks: [() -> Void] = []
    var pendingUnmountSeconds: TimeInterval = 0
    var unmountStarted: Date?
    var unmountFinished: Date?
    var stopObserved: Date?
    var localHost: [String: Any] = ["active": false, "phase": "idle"]
    var remoteHost: [String: Any] = ["active": false, "phase": "idle"]
    var localMount: [String: Any] = ["active": false, "phase": "idle"]
    var remoteMount: [String: Any] = ["active": false, "phase": "idle"]
    var afterRemoteStatus: (() -> Void)?

    private var fixtureController: PonteStorageController?
    private var fixtureView: NSView?
    private var fixtureTimer: Timer?
    private var watchdog: DispatchWorkItem?
    private var modalTicks = 0
    private var quitReply = false
    private(set) var fixtureFinished = false

    func state(_ phase: String, active: Bool) -> [String: Any] {
        ["session": sessionID, "share": "PDSD_" + sessionID,
         "phase": phase, "active": active, "read_only": true, "peer": "192.0.2.20"]
    }

    private func deliver(_ completion: @escaping ([String: Any]) -> Void,
                         _ result: [String: Any], afterwards: (() -> Void)? = nil) {
        let callback = { completion(result); afterwards?() }
        if holdCallbacks { deferredCallbacks.append(callback) }
        else if asyncCallbacks { DispatchQueue.main.async(execute: callback) }
        else { callback() }
    }

    func command(_ args: [String], completion: @escaping ([String: Any]) -> Void) {
        require(args.count >= 2 && args[0] == "storage", "Only fake storage commands are allowed")
        calls.append(args)
        let remote = args.contains("--remote")
        switch args[1] {
        case "list":
            deliver(completion, ["volumes": [["id": "fake-sd", "name": "Test SD",
                "bytes": 32_000_000_000, "removable": true],
                ["id": "fixed-disk", "name": "Must not appear", "bytes": 1, "removable": false]]])
        case "status":
            let callback = remote ? afterRemoteStatus : nil
            deliver(completion, ["host": remote ? remoteHost : localHost,
                                 "mount": remote ? remoteMount : localMount], afterwards: callback)
        case "publish":
            if malformedPublish {
                deliver(completion, ["ok": true, "session": "../invalid", "share": "other-share"])
                return
            }
            if remote { remoteHost = state("sharing", active: true) }
            else { localHost = state("sharing", active: true) }
            deliver(completion, ["ok": true, "session": sessionID,
                                 "share": "PDSD_" + sessionID, "phase": "preparing"])
        case "mount":
            if remote { remoteMount = state("mounted", active: true) }
            else { localMount = state("mounted", active: true) }
            deliver(completion, ["ok": true])
        case "unmount":
            unmountStarted = Date()
            if failUnmount { deliver(completion, ["error": "A file is still open"]); return }
            if pendingUnmountSeconds > 0 {
                if remote { remoteMount = state("unmounting", active: true) }
                else { localMount = state("unmounting", active: true) }
                DispatchQueue.main.asyncAfter(deadline: .now() + pendingUnmountSeconds) { [weak self] in
                    guard let self else { return }
                    if remote { self.remoteMount = self.state("unmounted", active: false) }
                    else { self.localMount = self.state("unmounted", active: false) }
                    self.unmountFinished = Date()
                }
                deliver(completion, ["ok": true, "phase": "pending"])
            } else {
                if remote { remoteMount = state("unmounted", active: false) }
                else { localMount = state("unmounted", active: false) }
                unmountFinished = Date()
                deliver(completion, ["ok": true])
            }
        case "stop":
            let destination = remote ? localMount : remoteMount
            require(destination["phase"] as? String == "unmounted", "Host stopped before destination confirmed unmounted")
            require(destination["session"] as? String == sessionID, "Stop used another mount session")
            stopObserved = Date()
            if remote { remoteHost = state("stopped", active: false) }
            else { localHost = state("stopped", active: false) }
            deliver(completion, ["ok": true])
        case "open": deliver(completion, ["ok": true])
        default:
            require(false, "Unexpected command; prepare/install must never run: \(args)")
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Recover an existing Mac -> Windows session, then terminate from a
        // run-loop block. This intentionally exercises AppKit's modal quit loop.
        asyncCallbacks = true
        pendingUnmountSeconds = 3
        localHost = state("sharing", active: true)
        remoteMount = state("mounted", active: true)
        let controller = PonteStorageController(owner: self)
        fixtureController = controller
        fixtureView = controller.makeView()
        let timer = Timer(timeInterval: 0.1, repeats: true) { [weak self] _ in
            guard let self else { return }
            if RunLoop.current.currentMode == .modalPanel { self.modalTicks += 1 }
            self.fixtureController?.tick()
        }
        fixtureTimer = timer
        RunLoop.main.add(timer, forMode: .default)
        RunLoop.main.add(timer, forMode: .modalPanel)
        afterRemoteStatus = { [weak self] in
            guard let self else { return }
            self.afterRemoteStatus = nil
            RunLoop.main.perform(inModes: [.default, .modalPanel, .eventTracking]) {
                NSApplication.shared.terminate(nil)
            }
        }
        let timeout = DispatchWorkItem {
            fputs("FAIL: windowless AppKit quit exceeded 15 seconds\n", stderr)
            fflush(stderr)
            exit(1)
        }
        watchdog = timeout
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 15, execute: timeout)
        controller.refreshAll()
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let fixtureController else { return .terminateCancel }
        fixtureController.prepareToQuit { [weak self] ready in
            DispatchQueue.main.async {
                guard let self else { return }
                require(ready, "Pending unmount caused quit to be cancelled")
                require(self.modalTicks > 0, "Storage timer did not execute in NSModalPanelRunLoopMode")
                guard let began = self.unmountStarted, let finished = self.unmountFinished,
                      let stopped = self.stopObserved else {
                    require(false, "Quit replied before complete unmount/stop evidence"); return
                }
                require(finished.timeIntervalSince(began) >= 2.9, "Fixture did not remain pending for more than two seconds")
                require(stopped >= finished, "Stop preceded completion of delayed unmount")
                require(self.remoteMount["phase"] as? String == "unmounted", "Destination still mounted at quit")
                require(self.localHost["phase"] as? String == "stopped", "Host still sharing at quit")
                self.quitReply = true
                sender.reply(toApplicationShouldTerminate: true)
            }
        }
        return .terminateLater
    }

    func applicationWillTerminate(_ notification: Notification) {
        require(quitReply, "Application terminated without the verified asynchronous reply")
        fixtureTimer?.invalidate()
        watchdog?.cancel()
        fixtureController?.invalidate()
        fixtureFinished = true
        print("PASS: real windowless NSApplication, modal timer, >2s pending unmount, asynchronous quit")
        fflush(stdout)
    }
}

private struct Fixture {
    let owner: AppDelegate
    let controller: PonteStorageController
    let view: NSView
    var picker: NSPopUpButton { descendants(view).compactMap { $0 as? NSPopUpButton }.first! }
    var direction: NSSegmentedControl { descendants(view).compactMap { $0 as? NSSegmentedControl }.first! }
    init() {
        owner = AppDelegate()
        controller = PonteStorageController(owner: owner)
        view = controller.makeView()
    }
    func action(_ name: String) {
        let selector = NSSelectorFromString(name)
        require(controller.responds(to: selector), "Production controller lost action \(name)")
        _ = controller.perform(selector)
    }
    func selectCard(remote: Bool = false) {
        controller.refreshAll()
        if remote { direction.selectedSegment = 1; action("sourceChanged") }
        require(picker.numberOfItems == 2, "Catalog must include only placeholder and removable card")
        picker.selectItem(at: 1)
        action("selectionChanged")
    }
}

private func testDirectionsAndRetry() {
    for remote in [false, true] {
        let fixture = Fixture()
        fixture.selectCard(remote: remote)
        fixture.action("startSharing")
        let publish = fixture.owner.calls.first { $0[1] == "publish" }!
        let mount = fixture.owner.calls.first { $0[1] == "mount" }!
        require(publish.contains("--remote") == remote, "Publish routed to wrong source")
        require(publish.contains("--parent") == !remote, "GUI parent must be attached only to local publish")
        require(mount.contains("--remote") == !remote, "Mount routed to wrong destination")
        require(mount.contains("--share") && mount.contains("PDSD_" + fixture.owner.sessionID), "Mount lost validated share")
        fixture.owner.failUnmount = true
        let failedStart = fixture.owner.calls.count
        fixture.action("stopSharing")
        require(!fixture.owner.calls.dropFirst(failedStart).contains { $0[1] == "stop" }, "Unmount error must preserve sharing")
        let host = remote ? fixture.owner.remoteHost : fixture.owner.localHost
        require(host["active"] as? Bool == true, "Unmount error changed active host")
        fixture.owner.failUnmount = false
        let retryStart = fixture.owner.calls.count
        fixture.action("stopSharing")
        let retry = fixture.owner.calls.dropFirst(retryStart)
        let verbs = retry.map { $0[1] }
        require(verbs.firstIndex(of: "unmount")! < verbs.firstIndex(of: "stop")!, "Retry did not preserve unmount -> stop ordering")
        let stop = retry.first { $0[1] == "stop" }!
        require(stop.contains("--remote") == remote, "Stop routed to wrong source")
        fixture.controller.invalidate()
    }
    print("PASS: both directions, removable-only catalog, unmount failure preservation, retry ordering")
}

private func testInvalidSessionAndStaleCallback() {
    let invalid = Fixture()
    invalid.selectCard()
    invalid.owner.malformedPublish = true
    invalid.action("startSharing")
    require(!invalid.owner.calls.contains { $0[1] == "mount" || $0[1] == "stop" }, "Invalid session reached a follow-up command")
    invalid.controller.invalidate()

    let stale = Fixture()
    stale.owner.holdCallbacks = true
    stale.controller.refreshAll()
    let countBefore = stale.picker.numberOfItems
    stale.controller.invalidate()
    let callbacks = stale.owner.deferredCallbacks
    stale.owner.deferredCallbacks.removeAll()
    callbacks.forEach { $0() }
    require(stale.picker.numberOfItems == countBefore, "Invalidated catalog callback still mutated UI")
    print("PASS: malformed session rejected; invalidated callbacks ignored")
}

@main
private enum StorageUITests {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)
        testDirectionsAndRetry()
        testInvalidSessionAndStaleCallback()
        fflush(stdout)
        let delegate = AppDelegate()
        app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
        require(delegate.fixtureFinished, "NSApplication returned without completing modal-quit fixture")
    }
}
