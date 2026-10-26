// Ponte Desk native preferences. GPL-3.0-or-later.
import AppKit

private final class PonteSettingsDocument: NSView {
    override var isFlipped: Bool { true }
}

final class PonteSettingsController: NSWindowController, NSWindowDelegate {
    private weak var appOwner: AppDelegate?
    private var original: [String: Any]
    private let localName = NSTextField()
    private let peerName = NSTextField()
    private let position = NSPopUpButton()
    private let protectWindows = NSSwitch()
    private let protectMac = NSSwitch()
    private let webcam = NSSwitch()
    private let shared = NSSwitch()
    private let manageDisplays = NSSwitch()
    private let macMonitor = NSPopUpButton()
    private let windowsMonitor = NSPopUpButton()
    private let refresh = NSButton()
    private let identifyMac = NSButton()
    private let identifyWindows = NSButton()
    private let calibration = NSTextField(wrappingLabelWithString: "")
    private let feedback = NSTextField(wrappingLabelWithString: "")
    private let save = NSButton()
    private let cancel = NSButton()
    private var monitorControls: NSView?
    private var macInput: UInt32?
    private var pcInput: UInt32?
    private var didFetchMonitors = false
    private var fetching = false
    private var applying = false
    private var calibratedMacID: String?
    private var calibratedWindowsID: String?

    init(owner: AppDelegate) {
        self.appOwner = owner
        original = owner.options
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 720),
                              styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "Ajustes do Ponte Desk"
        window.isReleasedWhenClosed = false
        window.contentMinSize = NSSize(width: 560, height: 540)
        window.backgroundColor = .windowBackgroundColor
        super.init(window: window)
        window.delegate = self
        build(owner)
        load(owner)
        window.center()
        window.initialFirstResponder = localName
        window.recalculateKeyViewLoop()
        if shared.state == .on {refreshWindows()}
    }

    required init?(coder: NSCoder) { nil }

    func windowShouldClose(_ sender: NSWindow) -> Bool { !applying }

    private func text(_ value: String, size: CGFloat = 13, weight: NSFont.Weight = .regular,
                      secondary: Bool = false) -> NSTextField {
        let result = NSTextField(wrappingLabelWithString: value)
        result.font = .systemFont(ofSize: size, weight: weight)
        result.textColor = secondary ? .secondaryLabelColor : .labelColor
        result.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return result
    }

    private func row(_ views: [NSView], spacing: CGFloat = 12) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.spacing = spacing
        return stack
    }

    private func column(_ views: [NSView], spacing: CGFloat = 10) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = spacing
        stack.detachesHiddenViews = true
        for view in views { view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true }
        return stack
    }

    private func spacer() -> NSView {
        let view = NSView()
        view.setContentHuggingPriority(.defaultLow, for: .horizontal)
        return view
    }

    private func button(_ control: NSButton, _ title: String, action: Selector) {
        control.title = title
        control.bezelStyle = .rounded
        control.target = self
        control.action = action
        control.setAccessibilityLabel(title)
        control.setContentHuggingPriority(.required, for: .horizontal)
    }

    private func switchRow(_ control: NSSwitch, title: String, explanation: String) -> NSView {
        control.target = self
        control.action = #selector(changedSwitch)
        control.setAccessibilityLabel(title)
        control.toolTip = explanation
        control.setContentHuggingPriority(.required, for: .horizontal)
        return row([column([text(title, weight: .medium), text(explanation, size: 12, secondary: true)], spacing: 3), control])
    }

    private func field(_ title: String, control: NSView) -> NSView {
        control.setAccessibilityLabel(title)
        return column([text(title, size: 12, secondary: true), control], spacing: 4)
    }

    private func section(_ title: String, _ views: [NSView]) -> NSView {
        let separator = NSBox()
        separator.boxType = .separator
        return column([text(title, size: 15, weight: .semibold)] + views + [separator], spacing: 14)
    }

    private func build(_ owner: AppDelegate) {
        guard let window else { return }
        let root = NSView()
        window.contentView = root
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.borderType = .noBorder
        scroll.translatesAutoresizingMaskIntoConstraints = false
        let document = PonteSettingsDocument()
        document.translatesAutoresizingMaskIntoConstraints = false
        scroll.documentView = document
        root.addSubview(scroll)

        localName.placeholderString = "Este Mac"
        peerName.placeholderString = "Outro computador"
        let names = row([field("Nome deste Mac", control: localName), field("Nome do outro computador", control: peerName)])
        names.arrangedSubviews[0].widthAnchor.constraint(equalTo: names.arrangedSubviews[1].widthAnchor).isActive = true
        for (title, value) in [("À esquerda", "left"), ("À direita", "right"), ("Acima", "top"), ("Abaixo", "bottom")] {
            position.addItem(withTitle: title)
            position.lastItem?.representedObject = value
        }
        let computers = section("Computadores", [names, field("Posição do outro computador em relação a este Mac", control: position)])
        let input = section("Teclado e mouse", [
            switchRow(protectWindows, title: "Proteger tela cheia no Windows",
                      explanation: "Pausa o teclado e o mouse entre os dois computadores enquanto o Windows está em tela cheia."),
            switchRow(protectMac, title: "Manter o cursor no Mac em tela cheia",
                      explanation: "Impede que o cursor saia deste Mac. O outro computador ainda pode controlar o Mac.")
        ])
        let camera = section("Webcam", [switchRow(webcam, title: "Disponibilizar compartilhamento de webcam",
            explanation: "Mostra os controles de webcam externa no painel. A câmera só inicia quando você escolher compartilhar.")])

        for popup in [macMonitor, windowsMonitor] {
            popup.target = self
            popup.action = #selector(monitorSelectionChanged)
        }
        button(refresh, "Atualizar monitores", action: #selector(refreshWindows))
        button(identifyMac, "Identificar entrada do Mac", action: #selector(identifyMacInput))
        button(identifyWindows, "Identificar entrada do Windows", action: #selector(identifyWindowsInput))
        calibration.font = .systemFont(ofSize: 12)
        calibration.textColor = .secondaryLabelColor
        let monitorFields = column([
            field("Monitor externo no Mac", control: macMonitor),
            field("Mesmo monitor no Windows", control: windowsMonitor),
            row([refresh, spacer()]),
            text("Selecione manualmente a entrada do Mac no próprio monitor e clique em Identificar entrada do Mac. Depois selecione a entrada do Windows e identifique-a. O Ponte Desk não muda a entrada física.", size: 12, secondary: true),
            row([identifyMac, identifyWindows, spacer()], spacing: 8),
            calibration,
            switchRow(manageDisplays, title: "Ajustar telas do Mac automaticamente",
                      explanation: "Reorganiza as telas do Mac quando o monitor está em uso pelo Windows. Requer uma tela local disponível.")
        ], spacing: 10)
        monitorControls = monitorFields
        let monitorSection = section("Monitor compartilhado", [
            switchRow(shared, title: "Uso o mesmo monitor nos dois computadores",
                      explanation: "Ative apenas se o monitor está conectado aos dois computadores e você alterna suas entradas."),
            monitorFields
        ])
        let permissions = NSButton(title: "Permissões do Mac…", target: owner, action: #selector(AppDelegate.openPermissions))
        permissions.bezelStyle = .rounded
        let restore = NSButton(title: "Restaurar telas do Mac", target: owner, action: #selector(AppDelegate.restoreDisplays))
        restore.bezelStyle = .rounded
        let tools = row([permissions, restore, spacer()], spacing: 8)
        let content = column([
            text("Seu espaço, do seu jeito", size: 22, weight: .semibold),
            text("Escolha como os dois computadores trabalham juntos.", secondary: true),
            computers, input, camera, monitorSection, tools
        ], spacing: 18)
        content.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(content)

        feedback.font = .systemFont(ofSize: 12)
        feedback.textColor = .secondaryLabelColor
        button(cancel, "Cancelar", action: #selector(cancelChanges))
        cancel.keyEquivalent = "\u{1b}"
        button(save, "Salvar", action: #selector(saveChanges))
        save.keyEquivalent = "\r"
        let bottom = column([feedback, row([spacer(), cancel, save], spacing: 8)], spacing: 7)
        bottom.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(bottom)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: root.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottom.topAnchor, constant: -12),
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            content.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 24),
            content.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -24),
            content.topAnchor.constraint(equalTo: document.topAnchor, constant: 24),
            content.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -20),
            bottom.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 24),
            bottom.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -24),
            bottom.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -18)
        ])
    }

    private func load(_ owner: AppDelegate) {
        localName.stringValue = original["local_name"] as? String ?? "Este Mac"
        peerName.stringValue = original["peer_name"] as? String ?? "Outro computador"
        if let item = position.itemArray.first(where: { $0.representedObject as? String == owner.peerPosition }) { position.select(item) }
        protectWindows.state = (original["protect_windows_fullscreen"] as? Bool ?? true) ? .on : .off
        protectMac.state = (original["protect_mac_fullscreen"] as? Bool ?? true) ? .on : .off
        webcam.state = (original["webcam_enabled"] as? Bool ?? true) ? .on : .off
        shared.state = (original["shared_monitor"] as? Bool ?? false) ? .on : .off
        manageDisplays.state = (original["manage_displays"] as? Bool ?? false) ? .on : .off
        let macID = original["mac_monitor_uuid"] as? String
        let pcID = original["windows_monitor"] as? String
        populate(macMonitor, monitors: owner.availableMacMonitors(), selected: macID)
        // Keep an existing selection while a network lookup is pending or fails.
        populate(windowsMonitor, monitors: pcID.map { [["id": $0, "name": "Monitor configurado"]] } ?? [], selected: pcID)
        macInput = Self.inputValue(original["mac_input"])
        pcInput = Self.inputValue(original["pc_input"])
        calibratedMacID = macID
        calibratedWindowsID = pcID
        if selectedID(macMonitor) != macID { clearCalibration() }
        updateControls()
        updateCalibration()
    }

    private func selectedID(_ popup: NSPopUpButton) -> String? { popup.selectedItem?.representedObject as? String }

    private func populate(_ popup: NSPopUpButton, monitors: [[String: Any]], selected: String?) {
        popup.removeAllItems()
        popup.addItem(withTitle: "Selecione um monitor")
        for monitor in monitors {
            guard let id = monitor["id"] as? String, !id.isEmpty else { continue }
            popup.addItem(withTitle: monitor["name"] as? String ?? "Monitor externo")
            popup.lastItem?.representedObject = id
            if id == selected { popup.select(popup.lastItem) }
        }
    }

    private static func inputValue(_ value: Any?) -> UInt32? {
        guard let number = value as? NSNumber, number.int64Value > 0,
              number.uint64Value <= UInt64(UInt32.max) else { return nil }
        let input = number.uint32Value & 0xff
        return input == 0 ? nil : input
    }

    private func updateControls() {
        monitorControls?.isHidden = shared.state != .on
        for control: NSControl in [localName, peerName, position, protectWindows, protectMac, webcam, shared] { control.isEnabled = !applying }
        for control: NSControl in [macMonitor, windowsMonitor, refresh, identifyMac, identifyWindows, manageDisplays] {
            control.isEnabled = !applying && !fetching && shared.state == .on
        }
        let selected = selectedID(macMonitor) != nil && selectedID(windowsMonitor) != nil
        identifyMac.isEnabled = identifyMac.isEnabled && selected
        identifyWindows.isEnabled = identifyWindows.isEnabled && selected
        save.isEnabled = !applying && !(fetching && shared.state == .on)
        cancel.isEnabled = !applying
        window?.standardWindowButton(.closeButton)?.isEnabled = !applying
    }

    private func message(_ value: String, error: Bool = false) {
        feedback.stringValue = value
        feedback.textColor = error ? .systemRed : .secondaryLabelColor
    }

    private func clearCalibration() {
        macInput = nil
        pcInput = nil
        calibratedMacID = selectedID(macMonitor)
        calibratedWindowsID = selectedID(windowsMonitor)
        updateCalibration()
    }

    private func updateCalibration() {
        let mac = macInput.map { "Mac: entrada \($0) identificada" } ?? "Mac: entrada ainda não identificada"
        let pc = pcInput.map { "Windows: entrada \($0) identificada" } ?? "Windows: entrada ainda não identificada"
        calibration.stringValue = mac + "\n" + pc
    }

    @objc private func changedSwitch() { updateControls();if shared.state == .on && !didFetchMonitors {refreshWindows()} }

    @objc private func monitorSelectionChanged() {
        clearCalibration()
        message("Monitor alterado. Identifique novamente as duas entradas.")
        updateControls()
    }

    @objc private func refreshWindows() {
        didFetchMonitors=true
        guard !fetching, !applying, let owner = appOwner else { return }
        let selectedMac = selectedID(macMonitor)
        populate(macMonitor, monitors: owner.availableMacMonitors(), selected: selectedMac)
        if selectedID(macMonitor) != selectedMac { clearCalibration() }
        let selected = selectedID(windowsMonitor)
        fetching = true
        updateControls()
        message("Buscando monitores do outro computador…")
        owner.fetchMonitors { [weak self] monitors, error in
            DispatchQueue.main.async {
                guard let self else { return }
                self.fetching = false
                if let error { self.message(error, error: true) }
                else {
                    self.populate(self.windowsMonitor, monitors: monitors, selected: selected)
                    if self.selectedID(self.windowsMonitor) != selected { self.clearCalibration() }
                    self.message(monitors.isEmpty ? "Nenhum monitor disponível no outro computador." : "Monitores atualizados.")
                }
                self.updateControls()
            }
        }
    }

    @objc private func identifyMacInput() { identify(mac: true) }
    @objc private func identifyWindowsInput() { identify(mac: false) }

    private func identify(mac: Bool) {
        guard !fetching, !applying, let owner = appOwner,
              let macID = selectedID(macMonitor), let pcID = selectedID(windowsMonitor) else { return }
        fetching = true
        updateControls()
        message("Lendo a entrada atualmente selecionada no monitor…")
        owner.fetchMonitors { [weak self] monitors, error in
            DispatchQueue.main.async {
                guard let self else { return }
                self.fetching = false
                defer { self.updateControls() }
                guard self.selectedID(self.macMonitor) == macID, self.selectedID(self.windowsMonitor) == pcID else { return }
                if let error { self.message(error, error: true); return }
                guard let monitor = monitors.first(where: { $0["id"] as? String == pcID }),
                      let value = Self.inputValue(monitor["input"]) else {
                    self.message("Não foi possível ler a entrada desse monitor. Confira a conexão e tente novamente.", error: true)
                    return
                }
                self.calibratedMacID = macID
                self.calibratedWindowsID = pcID
                if mac { self.macInput = value } else { self.pcInput = value }
                self.updateCalibration()
                if self.macInput == self.pcInput {
                    self.message("As duas entradas são iguais. Troque a entrada no próprio monitor e identifique novamente.", error: true)
                } else {
                    self.message("Entrada do \(mac ? "Mac" : "Windows") identificada: \(value).")
                }
            }
        }
    }

    @objc private func cancelChanges() { if !applying { close() } }

    @objc private func saveChanges() {
        guard !applying, let owner = appOwner else { return }
        let local = localName.stringValue.trimmingCharacters(in: .whitespaces)
        let peer = peerName.stringValue.trimmingCharacters(in: .whitespaces)
        guard [local, peer].allSatisfy({ !$0.isEmpty && $0.count <= 64 && $0.rangeOfCharacter(from: .newlines) == nil }) else {
            message("Use nomes de 1 a 64 caracteres, sem quebras de linha.", error: true)
            return
        }
        let useShared = shared.state == .on
        if useShared {
            guard !fetching, let macID = selectedID(macMonitor), let pcID = selectedID(windowsMonitor),
                  calibratedMacID == macID, calibratedWindowsID == pcID,
                  let macInput, let pcInput, macInput != pcInput else {
                message("Selecione o mesmo monitor nos dois computadores e identifique duas entradas diferentes antes de salvar.", error: true)
                return
            }
        }
        var values = original
        values["local_name"] = local
        values["peer_name"] = peer
        values["protect_windows_fullscreen"] = protectWindows.state == .on
        values["protect_mac_fullscreen"] = protectMac.state == .on
        values["webcam_enabled"] = webcam.state == .on
        values["shared_monitor"] = useShared
        values["manage_displays"] = useShared && manageDisplays.state == .on
        values["mac_monitor_uuid"] = selectedID(macMonitor) ?? ""
        values["windows_monitor"] = selectedID(windowsMonitor) ?? ""
        values["mac_input"] = macInput ?? 0
        values["pc_input"] = pcInput ?? 0
        let side = position.selectedItem?.representedObject as? String ?? "left"
        applying = true
        updateControls()
        message("Aplicando ajustes…")
        owner.saveOptions(values, position: side) { [weak self] error in
            DispatchQueue.main.async {
                guard let self else { return }
                self.applying = false
                self.updateControls()
                if let error { self.message(error, error: true) }
                else { self.close() }
            }
        }
    }
}
