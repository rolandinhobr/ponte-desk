// Ponte Desk native interface. GPL-3.0-or-later.
import AppKit

/// Drawing with semantic colours keeps surfaces responsive to appearance
/// changes without caching a light-mode CGColor on a backing layer.
private final class PontePanelBackground: NSView {
    override var isOpaque: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        bounds.fill()
    }
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }
}

private final class PontePanelDocument: NSView {
    override var isFlipped: Bool { true }
}

private final class PonteSurface: NSView {
    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        let shape = NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: 12, yRadius: 12)
        NSColor.controlBackgroundColor.setFill()
        shape.fill()
        NSColor.separatorColor.withAlphaComponent(0.5).setStroke()
        shape.lineWidth = 1
        shape.stroke()
    }
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }
}

private enum PonteLayout {
    static func label(_ text: String, size: CGFloat = 13, weight: NSFont.Weight = .regular,
                      secondary: Bool = false, wrapping: Bool = false) -> NSTextField {
        let field = wrapping ? NSTextField(wrappingLabelWithString: text) : NSTextField(labelWithString: text)
        field.font = .systemFont(ofSize: size, weight: weight)
        field.textColor = secondary ? .secondaryLabelColor : .labelColor
        field.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        return field
    }

    static func spacer() -> NSView {
        let view = NSView()
        view.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        view.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        return view
    }

    static func row(_ views: [NSView], spacing: CGFloat = 10) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.distribution = .fill
        stack.spacing = spacing
        stack.detachesHiddenViews = true
        return stack
    }

    static func column(_ views: [NSView], spacing: CGFloat = 8) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = spacing
        stack.detachesHiddenViews = true
        for view in views { view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true }
        return stack
    }

    static func card(_ views: [NSView], spacing: CGFloat = 8) -> NSView {
        let surface = PonteSurface()
        let content = column(views, spacing: spacing)
        content.translatesAutoresizingMaskIntoConstraints = false
        surface.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: surface.leadingAnchor, constant: 16),
            content.trailingAnchor.constraint(equalTo: surface.trailingAnchor, constant: -16),
            content.topAnchor.constraint(equalTo: surface.topAnchor, constant: 16),
            content.bottomAnchor.constraint(equalTo: surface.bottomAnchor, constant: -16)
        ])
        return surface
    }

    static func button(_ title: String, owner: AppDelegate, action: Selector, small: Bool = false) -> NSButton {
        let button = NSButton(title: title, target: owner, action: action)
        button.bezelStyle = .rounded
        button.controlSize = small ? .small : .regular
        button.setContentHuggingPriority(.required, for: .horizontal)
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        button.setAccessibilityLabel(title)
        return button
    }

    static func toggle(_ label: String, owner: AppDelegate, action: Selector) -> NSSwitch {
        let control = NSSwitch()
        control.target = owner
        control.action = action
        control.setAccessibilityLabel(label)
        control.setContentHuggingPriority(.required, for: .horizontal)
        return control
    }
}

/// Builds controls only. The owner applies observed state and visibility and
/// owns all camera, permission, service and configuration operations.
func buildPontePanel(_ owner: AppDelegate) -> NSWindow {
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 620, height: 720),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable],
                          backing: .buffered, defer: false)
    window.title = "Ponte Desk"
    window.isReleasedWhenClosed = false
    window.contentMinSize = NSSize(width: 580, height: 500)
    window.backgroundColor = .windowBackgroundColor
    window.center()

    let root = PontePanelBackground()
    window.contentView = root
    let mark = NSImageView(image: PonteArtwork.badge(size: 44))
    mark.setAccessibilityElement(false)
    mark.widthAnchor.constraint(equalToConstant: 44).isActive = true
    mark.heightAnchor.constraint(equalToConstant: 44).isActive = true
    let title = PonteLayout.label("Ponte Desk", size: 21, weight: .semibold)
    let status = PonteLayout.label("Verificando conexão…", size: 12, secondary: true, wrapping: true)
    owner.statusText = status
    let heading = PonteLayout.column([title, status], spacing: 2)
    heading.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
    status.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    let settings = PonteLayout.button("Ajustes…", owner: owner, action: #selector(AppDelegate.showSettings))
    let header = PonteLayout.row([mark, heading, settings], spacing: 12)
    // Fill the horizontal row explicitly; labels should receive the free width
    // instead of leaving unused gravity-area space beside the settings button.
    NSLayoutConstraint.activate([
        settings.trailingAnchor.constraint(equalTo: header.trailingAnchor),
        heading.trailingAnchor.constraint(equalTo: settings.leadingAnchor, constant: -12)
    ])

    let inputSwitch = PonteLayout.toggle("Compartilhar teclado e mouse", owner: owner,
                                         action: #selector(AppDelegate.togglePause))
    owner.inputSwitch = inputSwitch
    let inputHeading = PonteLayout.row([
        PonteLayout.label("Teclado e mouse", size: 14, weight: .semibold), PonteLayout.spacer(), inputSwitch
    ])
    let localName = PonteLayout.label(owner.localDisplayName, weight: .medium)
    let peerName = PonteLayout.label(owner.peerDisplayName, weight: .medium)
    localName.lineBreakMode = .byTruncatingTail
    peerName.lineBreakMode = .byTruncatingTail
    localName.toolTip = owner.localDisplayName
    peerName.toolTip = owner.peerDisplayName
    for name in [localName, peerName] {
        name.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        name.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
    }
    let connection = NSImageView(image: PonteArtwork.menuIcon())
    connection.contentTintColor = .secondaryLabelColor
    connection.setAccessibilityElement(false)
    connection.widthAnchor.constraint(equalToConstant: 24).isActive = true
    let computerRow = PonteLayout.row([localName, connection, peerName, PonteLayout.spacer()])
    let inputState = PonteLayout.label("Aguardando o outro computador", size: 12, secondary: true, wrapping: true)
    owner.inputState = inputState
    let layout = PonteLayout.label("", size: 11, secondary: true, wrapping: true)
    layout.isHidden = true
    owner.layoutLabel = layout
    let detail = PonteLayout.label("", size: 11, secondary: true, wrapping: true)
    detail.isHidden = true
    owner.detailText = detail
    let permissions = PonteLayout.button("Permissões…", owner: owner,
                                          action: #selector(AppDelegate.openPermissions), small: true)
    owner.permissionsButton=permissions
    permissions.toolTip = "Verificar as permissões de controle deste Mac"
    let inputMode=NSPopUpButton()
    inputMode.addItems(withTitles:["Passar pelas bordas", "Usar os periféricos dos dois neste Mac", "Usar os periféricos dos dois no Windows"])
    inputMode.target=owner;inputMode.action=#selector(AppDelegate.changeInputMode)
    inputMode.setAccessibilityLabel("Destino do teclado e mouse")
    owner.inputMode=inputMode
    let inputCard = PonteLayout.card([inputHeading, computerRow, inputMode,
        PonteLayout.row([inputState, PonteLayout.spacer(), permissions]), layout, detail], spacing: 8)

    let component = PonteLayout.button("Configurar câmera…", owner: owner,
                                        action: #selector(AppDelegate.installCameraDriver), small: true)
    owner.cameraComponentButton=component
    component.toolTip = "Configurar o componente de câmera virtual deste Mac"
    let cameraHeading = PonteLayout.row([
        PonteLayout.label("Webcam", size: 14, weight: .semibold), PonteLayout.spacer(), component
    ])
    let origin = NSSegmentedControl(labels: ["Este Mac", "Outro computador"], trackingMode: .selectOne,
                                   target: owner, action: #selector(AppDelegate.cameraOriginChanged))
    origin.selectedSegment = 0
    origin.segmentStyle = .rounded
    origin.setAccessibilityLabel("Computador ao qual a webcam está conectada")
    owner.cameraOrigin = origin
    let originRow = PonteLayout.row([PonteLayout.label("Conectada a", size: 12, secondary: true), origin, PonteLayout.spacer()])
    let local = NSPopUpButton()
    local.addItem(withTitle: "Nenhuma webcam externa disponível")
    local.setAccessibilityLabel("Webcam externa deste Mac")
    let remote = NSPopUpButton()
    remote.addItem(withTitle: "Atualize as câmeras do outro computador")
    remote.setAccessibilityLabel("Webcam externa do outro computador")
    remote.isHidden = true
    owner.cameraLocal = local
    owner.cameraRemote = remote
    let devices = PonteLayout.column([local, remote], spacing: 0)
    devices.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
    for popup in [local, remote] {
        popup.setContentHuggingPriority(.defaultLow, for: .horizontal)
        popup.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    }
    let resolution = NSPopUpButton()
    resolution.addItems(withTitles: ["1080p · 30 fps", "720p · 30 fps"])
    resolution.item(at: 0)?.representedObject = "1080p"
    resolution.item(at: 1)?.representedObject = "720p"
    if UserDefaults.standard.string(forKey: "webcamQuality") == "720p" { resolution.selectItem(at: 1) }
    resolution.setAccessibilityLabel("Qualidade de captura da webcam")
    resolution.toolTip = "Qualidade aplicada ao iniciar o compartilhamento. 720p usa menos rede."
    resolution.setContentHuggingPriority(.required, for: .horizontal)
    resolution.widthAnchor.constraint(equalToConstant: 150).isActive = true
    owner.cameraResolution = resolution
    let deviceRow = PonteLayout.row([devices, resolution], spacing: 8)
    let start = PonteLayout.button("Compartilhar no outro computador", owner: owner,
                                    action: #selector(AppDelegate.startSelectedWebcam))
    let stop = PonteLayout.button("Parar", owner: owner, action: #selector(AppDelegate.stopWebcam))
    stop.setAccessibilityLabel("Parar compartilhamento da webcam")
    stop.isEnabled = false
    let refresh = PonteLayout.button("Atualizar", owner: owner, action: #selector(AppDelegate.refreshCameras), small: true)
    refresh.setAccessibilityLabel("Atualizar lista de webcams externas")
    owner.cameraStart = start
    owner.cameraStop = stop
    let actions = PonteLayout.row([start, stop, PonteLayout.spacer(), refresh], spacing: 8)
    let cameraState = PonteLayout.label("Webcam desligada", size: 12, secondary: true, wrapping: true)
    owner.cameraState = cameraState
    let cameraHint = PonteLayout.label("No aplicativo de chamada, escolha Ponte Desk Camera", size: 11,
                                        secondary: true, wrapping: true)
    // The owner reveals this instruction only after confirming installation.
    cameraHint.isHidden = true
    owner.cameraNameHint = cameraHint
    let cameraCard = PonteLayout.card([cameraHeading, originRow, deviceRow, actions, cameraState, cameraHint], spacing: 9)

    let displaySwitch = PonteLayout.toggle("Ajustar telas automaticamente", owner: owner,
                                           action: #selector(AppDelegate.toggleDisplays))
    owner.autoDisplaySwitch = displaySwitch
    let monitorHeading = PonteLayout.row([
        PonteLayout.label("Monitor compartilhado", size: 14, weight: .semibold), PonteLayout.spacer(), displaySwitch
    ])
    displaySwitch.toolTip = "Ajustar as telas do Mac conforme a entrada selecionada no monitor"
    let monitorStatus = PonteLayout.label("Verificando monitor…", size: 12, secondary: true, wrapping: true)
    owner.monitorStatus = monitorStatus
    let restore = PonteLayout.button("Restaurar telas", owner: owner,
                                      action: #selector(AppDelegate.restoreDisplays), small: true)
    let monitorCard = PonteLayout.card([monitorHeading, PonteLayout.row([monitorStatus, restore])], spacing: 6)
    owner.monitorGroup = monitorCard

    let footer = PonteLayout.label("Controle local: Ctrl + Alt/Option + Esc", size: 11, secondary: true)
    footer.alignment = .center
    let storageCard = PonteLayout.card([owner.storage.makeView()])
    let stack = PonteLayout.column([inputCard, cameraCard, storageCard, monitorCard], spacing: 14)
    stack.translatesAutoresizingMaskIntoConstraints = false
    footer.translatesAutoresizingMaskIntoConstraints = false
    header.translatesAutoresizingMaskIntoConstraints = false
    let scroll = NSScrollView()
    scroll.translatesAutoresizingMaskIntoConstraints = false
    scroll.drawsBackground = false; scroll.hasVerticalScroller = true
    scroll.autohidesScrollers = true; scroll.borderType = .noBorder
    let document = PontePanelDocument()
    document.translatesAutoresizingMaskIntoConstraints = false
    scroll.documentView = document
    document.addSubview(stack)
    root.addSubview(header)
    root.addSubview(scroll)
    root.addSubview(footer)
    NSLayoutConstraint.activate([
        header.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 20),
        header.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -20),
        header.topAnchor.constraint(equalTo: root.topAnchor, constant: 18),
        scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
        scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
        scroll.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 16),
        scroll.bottomAnchor.constraint(equalTo: footer.topAnchor, constant: -14),
        document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
        stack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 20),
        stack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -20),
        stack.topAnchor.constraint(equalTo: document.topAnchor),
        stack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -8),
        footer.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 20),
        footer.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -20),
        footer.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -18)
    ])
    window.initialFirstResponder = inputSwitch
    window.recalculateKeyViewLoop()
    return window
}
