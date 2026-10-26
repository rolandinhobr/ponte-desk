// Manual, read-only removable-volume sharing. GPL-3.0-or-later.
import AppKit
import ServiceManagement

final class PonteStorageController: NSObject {
    private weak var owner: AppDelegate?
    private enum Phase { case idle, preparing, mounting, ready, unmounting, stopping, attention }
    private struct Session {
        let id: String
        let share: String
        let peer: String
        let sourceRemote: Bool
    }
    private var session: Session?
    private var phase: Phase = .idle
    private var host: [String: Any] = [:]
    private var mount: [String: Any] = [:]
    private var commandBusy = false
    private var statusBusy = false
    private var catalogBusy = false
    private var generation = 0
    private var catalogGeneration = 0
    private var lastPoll = Date.distantPast
    private var operationStarted = Date.distantPast
    private var observations: [NSObjectProtocol] = []
    private var quitCompletion: ((Bool) -> Void)?
    private var lastMessage = "Escolha um cartão para acessar seus arquivos em modo somente leitura."
    private var lastError = false
    private var source = NSSegmentedControl()
    private var volumes = NSPopUpButton()
    private var start = NSButton()
    private var stop = NSButton()
    private var open = NSButton()
    private var refresh = NSButton()
    private var prepare = NSButton()
    private var status = NSTextField(wrappingLabelWithString: "")

    init(owner: AppDelegate) {
        self.owner = owner
        super.init()
        if !owner.preview {
            let center = NSWorkspace.shared.notificationCenter
            for name in [NSWorkspace.didMountNotification, NSWorkspace.didUnmountNotification] {
                observations.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                    guard let self else { return }
                    if self.source.selectedSegment == 0 && self.session == nil { self.refreshCatalog() }
                    self.pollStatus(force: true)
                })
            }
        }
    }

    deinit { for observation in observations { NSWorkspace.shared.notificationCenter.removeObserver(observation) } }

    private func label(_ value: String, size: CGFloat = 12, bold: Bool = false) -> NSTextField {
        let view = NSTextField(wrappingLabelWithString: value)
        view.font = .systemFont(ofSize: size, weight: bold ? .semibold : .regular)
        view.textColor = bold ? .labelColor : .secondaryLabelColor
        return view
    }
    private func row(_ views: [NSView]) -> NSStackView {
        let stack = NSStackView(views: views); stack.orientation = .horizontal
        stack.alignment = .centerY; stack.distribution = .fill; stack.spacing = 8
        return stack
    }
    private func spacer() -> NSView {
        let view = NSView(); view.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal); return view
    }
    private func button(_ title: String, action: Selector, small: Bool = false) -> NSButton {
        let view = NSButton(title: title, target: self, action: action)
        view.bezelStyle = .rounded; view.controlSize = small ? .small : .regular
        view.setContentHuggingPriority(.required, for: .horizontal)
        view.setContentCompressionResistancePriority(.required, for: .horizontal)
        view.setAccessibilityLabel(title); return view
    }

    /// Pure view construction. Rebuilding the main panel preserves the session.
    func makeView() -> NSView {
        prepare = button("Configurar…", action: #selector(prepareMac), small: true)
        let heading = row([label("Cartão SD", size: 14, bold: true), spacer(), prepare])
        source = NSSegmentedControl(labels: ["Este Mac", "Outro computador"], trackingMode: .selectOne,
                                    target: self, action: #selector(sourceChanged))
        source.selectedSegment = session?.sourceRemote == true ? 1 : 0
        source.segmentStyle = .rounded
        source.setAccessibilityLabel("Computador ao qual o cartão está conectado")
        volumes = NSPopUpButton(); volumes.addItem(withTitle: "Atualize para localizar cartões removíveis")
        volumes.target = self; volumes.action = #selector(selectionChanged)
        volumes.setAccessibilityLabel("Cartão ou volume removível para compartilhar somente leitura")
        volumes.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        start = button("Compartilhar cartão", action: #selector(startSharing))
        stop = button("Desmontar e parar", action: #selector(stopSharing))
        open = button("Abrir no destino", action: #selector(openDestination), small: true)
        refresh = button("Atualizar", action: #selector(refreshAll), small: true)
        status = label(lastMessage)
        let stack = NSStackView(views: [heading,
            row([label("Conectado a"), source, spacer()]), volumes,
            row([start, stop, spacer(), refresh]), row([open, spacer()]), status,
            label("Somente leitura no outro computador. Para retirar o cartão, desmonte e pare primeiro.", size: 11)])
        stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 9
        for view in stack.arrangedSubviews { view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true }
        updateView()
        return stack
    }

    private var sourceRemote: Bool { session?.sourceRemote ?? (source.selectedSegment == 1) }
    private var operating: Bool { [.preparing, .mounting, .unmounting, .stopping].contains(phase) }
    private func updateView() {
        let preview = owner?.preview == true
        let busy = operating || commandBusy || catalogBusy
        source.isEnabled = !preview && !busy && session == nil
        volumes.isEnabled = !preview && !busy && session == nil
        prepare.isEnabled = !preview && !busy && session == nil
        prepare.isHidden = source.selectedSegment == 1
        refresh.isEnabled = !preview && !busy
        start.title = session == nil ? "Compartilhar cartão" : "Montar no destino"
        start.isEnabled = !preview && !busy && (session == nil ? volumes.selectedItem?.representedObject is String : host["phase"] as? String == "sharing" && host["read_only"] as? Bool == true && mount["phase"] as? String != "mounted")
        stop.isEnabled = !preview && !busy && session != nil
        open.isEnabled = !preview && !busy && session != nil && mount["phase"] as? String == "mounted" && mount["active"] as? Bool == true && mount["read_only"] as? Bool == true
        status.stringValue = lastMessage; status.textColor = lastError ? .systemRed : .secondaryLabelColor
    }
    private func message(_ value: String, error: Bool = false) { lastMessage = value; lastError = error; updateView() }
    private func fail(_ value: String) {
        phase = session == nil ? .idle : .attention
        message(value, error: true)
        if let completion = quitCompletion { quitCompletion = nil; completion(false) }
    }
    private static func validSession(_ value: String) -> Bool {
        value.count == 32 && value.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
    private func run(_ arguments: [String], completion: @escaping ([String: Any]) -> Void) {
        guard let owner, !owner.preview else { return }
        let token = generation
        owner.command(["storage"] + arguments) { [weak self] result in
            guard let self, self.generation == token else { return }
            completion(result)
        }
    }
    private func route(_ verb: String, remote: Bool, peer: String) -> [String] {
        remote ? [verb, "--remote", "--peer", peer] : [verb]
    }
    private func destination(_ verb: String, _ session: Session) -> [String] {
        var args = route(verb, remote: !session.sourceRemote, peer: session.peer)
        if session.sourceRemote { args += ["--peer", session.peer] }
        return args + ["--share", session.share, "--session", session.id]
    }

    @objc private func sourceChanged() { catalogGeneration += 1; catalogBusy = false; refreshCatalog() }
    @objc private func selectionChanged() { updateView() }
    @objc func refreshAll() {
        if phase == .attention { phase = .ready }
        refreshCatalog(); pollStatus(force: true)
    }

    private func refreshCatalog() {
        guard !catalogBusy, !commandBusy, !operating, session == nil, let owner, !owner.preview else { return }
        let remote = source.selectedSegment == 1
        guard !remote || owner.cameraPeer != nil else { message("Configure o outro computador antes de buscar cartões.", error: true); return }
        let selected = volumes.selectedItem?.representedObject as? String
        let token = catalogGeneration
        catalogBusy = true; updateView()
        run(route("list", remote: remote, peer: owner.cameraPeer ?? "")) { [weak self] result in
            guard let self, self.catalogGeneration == token else { return }
            self.catalogBusy = false
            if let error = result["error"] as? String { self.message(error, error: true); return }
            let available = (result["volumes"] as? [[String: Any]] ?? []).filter { $0["removable"] as? Bool == true }
            self.volumes.removeAllItems()
            self.volumes.addItem(withTitle: available.isEmpty ? "Nenhum cartão removível disponível" : "Selecione um cartão")
            for item in available {
                guard let id = item["id"] as? String, !id.isEmpty else { continue }
                let name = item["name"] as? String ?? "Cartão removível"
                let bytes = (item["bytes"] as? NSNumber)?.int64Value ?? 0
                let capacity = bytes > 0 ? " · " + ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file) : ""
                self.volumes.addItem(withTitle: name + capacity)
                self.volumes.lastItem?.representedObject = id
                if id == selected { self.volumes.select(self.volumes.lastItem) }
            }
            self.updateView()
        }
    }

    @objc private func prepareMac() {
        guard !commandBusy, session == nil else { return }
        commandBusy = true; message("Preparando o compartilhamento deste Mac…")
        run(["prepare"]) { [weak self] result in
            guard let self else { return }; self.commandBusy = false
            if let error = result["error"] as? String { self.fail(error); return }
            let detail = result["message"] as? String ?? "Ative Compartilhamento de Arquivos e habilite seu usuário nas opções SMB dos Ajustes do Sistema."
            self.message(detail, error: result["ready"] as? Bool != true)
            if result["approval_required"] as? Bool == true {
                self.message(detail + " Autorize também o auxiliar em Itens de Início de Sessão e Extensões.")
                SMAppService.openSystemSettingsLoginItems()
            } else if result["ready"] as? Bool == true,
                      let url=URL(string:"x-apple.systempreferences:com.apple.Sharing-Settings.extension") {
                NSWorkspace.shared.open(url)
            }
        }
    }

    @objc private func startSharing() {
        guard !commandBusy, !operating, let owner, !owner.preview else { return }
        if let session { beginMount(session); return }
        guard let id = volumes.selectedItem?.representedObject as? String, let peer = owner.cameraPeer else {
            message("Selecione um cartão e configure o outro computador.", error: true); return
        }
        let remote = source.selectedSegment == 1
        commandBusy = true; phase = .preparing; operationStarted = Date()
        message(remote ? "Preparando o cartão. Autorize a solicitação no Windows." : "Preparando acesso somente leitura ao cartão…")
        var args = route("publish", remote: remote, peer: peer) + ["--id", id]
        if !remote { args += ["--peer", peer, "--parent", String(ProcessInfo.processInfo.processIdentifier)] }
        run(args) { [weak self] result in
            guard let self else { return }; self.commandBusy = false
            if let error = result["error"] as? String { self.fail(error); self.pollStatus(force: true); return }
            guard let id = result["session"] as? String, Self.validSession(id),
                  let share = result["share"] as? String, share == "PDSD_" + id else {
                self.fail("O compartilhamento não retornou uma sessão válida. Atualize para conferir o estado."); return
            }
            self.session = Session(id: id, share: share, peer: peer, sourceRemote: remote)
            self.host = [:]; self.mount = [:]
            self.updateView(); self.pollStatus(force: true)
        }
    }

    private func beginMount(_ value: Session) {
        guard !commandBusy else { return }
        phase = .mounting; commandBusy = true; operationStarted = Date()
        message("Montando o cartão no destino, somente leitura…")
        run(destination("mount", value)) { [weak self] result in
            guard let self, self.session?.id == value.id else { return }; self.commandBusy = false
            if let error = result["error"] as? String { self.fail(error + " O compartilhamento foi preservado."); return }
            self.pollStatus(force: true)
        }
    }

    @objc private func stopSharing() {
        guard !commandBusy, !operating, let value = session else { return }
        phase = .unmounting; commandBusy = true; operationStarted = Date()
        message("Desmontando no destino antes de encerrar o compartilhamento…")
        run(destination("unmount", value)) { [weak self] result in
            guard let self, self.session?.id == value.id else { return }; self.commandBusy = false
            if let error = result["error"] as? String {
                self.fail(error + " Feche arquivos no destino e tente novamente. O compartilhamento foi preservado."); return
            }
            self.pollStatus(force: true)
        }
    }

    private func stopHost(_ value: Session) {
        // Only the observed destination state authorizes stopping a share.
        guard !commandBusy, mount["session"] as? String == value.id,
              mount["phase"] as? String == "unmounted", mount["active"] as? Bool != true else { return }
        phase = .stopping; commandBusy = true; operationStarted = Date()
        message("Cartão desmontado. Encerrando o compartilhamento…")
        run(route("stop", remote: value.sourceRemote, peer: value.peer) + ["--session", value.id]) { [weak self] result in
            guard let self, self.session?.id == value.id else { return }; self.commandBusy = false
            if let error = result["error"] as? String { self.fail(error); return }
            self.pollStatus(force: true)
        }
    }

    @objc private func openDestination() {
        guard let value = session, !commandBusy, mount["phase"] as? String == "mounted" else { return }
        commandBusy = true; updateView()
        run(route("open", remote: !value.sourceRemote, peer: value.peer) + ["--session", value.id]) { [weak self] result in
            guard let self else { return }; self.commandBusy = false
            if let error = result["error"] as? String { self.message(error, error: true) }
            else { self.updateView() }
        }
    }

    /// Called by the application's existing timer. No idle catalog polling.
    func tick() {
        guard session != nil || operating else { return }
        if operating && Date().timeIntervalSince(operationStarted) > 120 {
            if commandBusy {
                message("Ainda aguardando o sistema. Confira a autorização ou os arquivos abertos no destino. Nenhum compartilhamento será encerrado sem confirmar a desmontagem.")
                return
            }
            fail("A operação ainda não foi confirmada. Verifique autorizações ou arquivos abertos no destino e atualize. O compartilhamento foi preservado.")
        }
        pollStatus(force: false)
    }

    private func pollStatus(force: Bool) {
        guard !statusBusy, !commandBusy, let owner, !owner.preview else { return }
        guard force || Date().timeIntervalSince(lastPoll) >= 2 else { return }
        statusBusy = true; lastPoll = Date()
        let peer = session?.peer ?? owner.cameraPeer
        run(["status"]) { [weak self] local in
            guard let self else { return }
            guard let peer else {
                self.statusBusy = false
                self.reconcile(local: local, remote: [:], peer: nil); return
            }
            self.run(["status", "--remote", "--peer", peer]) { [weak self] remote in
                guard let self else { return }; self.statusBusy = false
                self.reconcile(local: local, remote: remote, peer: peer)
            }
        }
    }

    private func reconcile(local: [String: Any], remote: [String: Any], peer: String?) {
        if let error = local["error"] as? String ?? remote["error"] as? String {
            // With no known session and both local resources explicitly idle,
            // quitting cannot tear down a mounted card or a locally hosted share.
            let localStates = [local["host"], local["mount"]].compactMap { $0 as? [String: Any] }
            if quitCompletion != nil && session == nil && localStates.count == 2 && localStates.allSatisfy({
                $0["active"] as? Bool == false && ["idle", "stopped", "unmounted"].contains($0["phase"] as? String ?? "")
            }) {
                let completion = quitCompletion; quitCompletion = nil; completion?(true); return
            }
            if session != nil || quitCompletion != nil { fail("Estado não confirmado: " + error + " O compartilhamento foi preservado.") }
            else { message(error, error: true) }
            return
        }
        guard local["host"] is [String: Any], local["mount"] is [String: Any],
              peer == nil || (remote["host"] is [String: Any] && remote["mount"] is [String: Any]) else {
            fail("O estado do cartão está incompleto. Atualize antes de continuar."); return
        }
        let localHost = local["host"] as? [String: Any] ?? [:]
        let remoteHost = remote["host"] as? [String: Any] ?? [:]
        let localMount = local["mount"] as? [String: Any] ?? [:]
        let remoteMount = remote["mount"] as? [String: Any] ?? [:]
        func relevant(_ state: [String: Any]) -> Bool {
            state["active"] as? Bool == true || ["preparing", "sharing", "mounting", "unmounting"].contains(state["phase"] as? String ?? "")
        }
        var recovered = false
        if session == nil {
            let candidates: [(Bool, [String: Any])] = [(false, localHost), (true, remoteHost), (true, localMount), (false, remoteMount)]
            if candidates.contains(where: { relevant($0.1) && !Self.validSession($0.1["session"] as? String ?? "") }) {
                fail("Há uma sessão de cartão com identificação inválida. Não foi possível encerrá-la com segurança."); return
            }
            let valid = candidates.filter { relevant($0.1) && Self.validSession($0.1["session"] as? String ?? "") }
            let ids = Set(valid.compactMap { $0.1["session"] as? String })
            if ids.count > 1 { fail("Há mais de uma sessão de cartão ativa. Confira os dois computadores antes de continuar."); return }
            if let candidate = valid.first, let peer,
               let id = candidate.1["session"] as? String, let share = candidate.1["share"] as? String, share == "PDSD_" + id {
                session = Session(id: id, share: share, peer: peer, sourceRemote: candidate.0)
                source.selectedSegment = candidate.0 ? 1 : 0
                phase = .attention
                recovered = true
            }
        }
        guard let value = session else {
            if let completion = quitCompletion { quitCompletion = nil; completion(true) }
            updateView(); return
        }
        let seenHost = value.sourceRemote ? remoteHost : localHost
        let seenMount = value.sourceRemote ? localMount : remoteMount
        // Never carry over a terminal state from a different session.
        host = seenHost["session"] as? String == value.id ? seenHost : [:]
        mount = seenMount["session"] as? String == value.id ? seenMount : [:]
        let hostPhase = host["phase"] as? String ?? ""
        let mountPhase = mount["phase"] as? String ?? ""
        if commandBusy { updateView(); return }
        let cleaning = phase == .unmounting || phase == .stopping
        if !cleaning && hostPhase == "sharing" && host["read_only"] as? Bool != true {
            fail("O acesso somente leitura não foi confirmado. Não monte o cartão; desmonte e pare para conferir a configuração."); return
        }
        if !cleaning && mountPhase == "mounted" && mount["read_only"] as? Bool != true {
            fail("O destino não confirmou acesso somente leitura. Desmonte e pare antes de continuar."); return
        }
        if hostPhase == "stopped" && mountPhase == "unmounted" {
            session = nil; phase = .idle; message("Cartão desmontado. Compartilhamento encerrado.")
            if let completion = quitCompletion { quitCompletion = nil; completion(true) }
            return
        }
        if phase == .preparing && hostPhase == "sharing" { beginMount(value); return }
        if phase == .unmounting && mountPhase == "unmounted" {
            if hostPhase == "stopped" { session = nil; phase = .idle }
            else { stopHost(value) }
            return
        }
        if hostPhase == "error" || mountPhase == "error" {
            let error = (phase == .unmounting ? mount["error"] : host["error"]) as? String ?? mount["error"] as? String ?? "O sistema não concluiu a operação."
            fail(error + " O compartilhamento não foi encerrado automaticamente."); return
        }
        // Keep an actionable failure visible until the person refreshes or
        // retries; successful background samples must not erase that message.
        if phase == .attention && !recovered { updateView(); return }
        if mountPhase == "mounted" && hostPhase == "sharing" && ![Phase.unmounting, .stopping].contains(phase) {
            phase = .ready; message("Cartão disponível no destino · somente leitura")
        } else if !operating {
            message(hostPhase == "sharing" ? "Cartão compartilhado. Monte no destino ou desmonte e pare." : "Sessão recuperada. Atualize para confirmar o estado antes de retirar o cartão.")
        }
        if quitCompletion != nil && !operating { stopSharing() }
        updateView()
    }

    /// Normal quit preserves the unmount-before-stop ordering. Crash cleanup is
    /// owned by the native host's parent-process watchdog, not by this UI.
    func prepareToQuit(_ completion: @escaping (Bool) -> Void) {
        guard owner?.preview != true else { completion(true); return }
        guard quitCompletion == nil else { completion(false); return }
        quitCompletion = completion
        if operating || commandBusy {
            quitCompletion = nil; message("Aguarde a operação do cartão terminar antes de encerrar.", error: true); completion(false); return
        }
        if session != nil { stopSharing() } else { pollStatus(force: true) }
    }

    func invalidate() {
        generation += 1; catalogGeneration += 1
        for observation in observations { NSWorkspace.shared.notificationCenter.removeObserver(observation) }
        observations.removeAll(); quitCompletion = nil
    }
}
