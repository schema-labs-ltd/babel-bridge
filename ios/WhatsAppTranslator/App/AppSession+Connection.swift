import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func start() async {
        if ProcessInfo.processInfo.arguments.contains("-demoOnboarding") {
            phase = .needsConfiguration
            return
        }
        if demoMode {
            loadDemoData()
            #if DEBUG
            if ProcessInfo.processInfo.arguments.contains("-demoRelinking") {
                Task {
                    try? await Task.sleep(for: .seconds(2))
                    handle(LiveEvent.signal("disconnected"))
                    let status = try! JSONDecoder().decode(LiveEvent.self, from: Data(#"{"type":"status","connected":false,"connection_state":"linking_required","qr":null}"#.utf8))
                    handle(status)
                    try? await Task.sleep(for: .seconds(1))
                    var code = LiveEvent.signal("qr")
                    code.data = "BABEL-BRIDGE-RELINK-PREVIEW-NOT-A-LIVE-CODE"
                    handle(code)
                    try? await Task.sleep(for: .seconds(10))
                    var connected = LiveEvent.signal("connected")
                    connected.name = "Preview"
                    handle(connected)
                }
            }
            #endif
            return
        }
        guard let stored = credentials.load() else {
            phase = .needsConfiguration
            return
        }
        await connect(configuration: stored, remember: false, restoreCached: true)
    }

    func connect(address: String, password: String) async {
        do {
            let configuration = try ServerConfiguration.make(address: address, password: password)
            await connect(configuration: configuration, remember: true)
        } catch {
            presentError("Couldn’t connect", error)
            phase = .needsConfiguration
        }
    }

    /// Refreshes the chat list. Dropped connections (for example a request cut off when the
    /// phone locks, or made before the network is back after unlocking) are retried quietly.
    /// Automatic refreshes never alert for those, because the cached chats stay on screen and
    /// the live connection keeps retrying; a pull to refresh reports them once retries run out.
    func refresh(reportTransientFailures: Bool = false) async {
        guard !demoMode else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        var retryDelays = transientRefreshRetryDelays[...]
        while true {
            let revision = connectionRevision
            do {
                async let status = api.status()
                async let contacts = api.contacts()
                let latestStatus = try await status
                if revision == connectionRevision { applyBackendStatus(latestStatus) }
                self.contacts = try await contacts
                await persistCache()
                return
            } catch {
                guard !Self.isExpectedCancellation(error) else { return }
                guard Self.isTransientNetworkError(error) else {
                    presentError("Couldn’t refresh chats", error)
                    return
                }
                guard let delay = retryDelays.popFirst() else {
                    if reportTransientFailures { presentError("Couldn’t refresh chats", error) }
                    return
                }
                do { try await Task.sleep(for: delay) } catch { return }
            }
        }
    }

    func applyBackendStatus(_ status: BackendStatus) {
        connectionRevision = UUID()
        backendStatus = status
        if status.connected || status.connectionState != nil { whatsAppQRCode = status.connected ? nil : status.qr }
        whatsAppLinkError = nil
    }

    func refreshWhatsAppConnection() async {
        guard !demoMode, configuration != nil, phase == .ready else { return }
        let revision = connectionRevision
        do {
            let status = try await api.status()
            guard revision == connectionRevision, phase == .ready, !Task.isCancelled else { return }
            applyBackendStatus(status)
            // Compatibility with servers that predate the connection-state snapshot.
            if !status.connected && status.connectionState == nil {
                let qrRevision = connectionRevision
                let response = try await api.whatsAppQRCode()
                guard qrRevision == connectionRevision, phase == .ready, !Task.isCancelled else { return }
                whatsAppQRCode = response.qr
            }
        } catch {
            guard revision == connectionRevision, !Self.isExpectedCancellation(error) else { return }
            whatsAppLinkError = "The server is unavailable. We’ll keep checking automatically."
        }
    }

    func monitorWhatsAppConnection() async {
        guard !demoMode else { return }
        while !Task.isCancelled && phase == .ready {
            await refreshWhatsAppConnection()
            do { try await Task.sleep(for: .seconds(backendStatus.connected ? 20 : 5)) }
            catch { return }
        }
    }

    func forgetServer() {
        applyBackendStatus(BackendStatus(connected: false, phone: nil, name: nil, connectionState: "connecting"))
        #if os(iOS)
        MessageVideoPlayback.stopAll()
        #endif
        savedConversationSettings = [:]
        recoveryTask?.cancel()
        liveUpdatesReconnecting = false
        Task {
            await api.disconnectLiveEvents()
            await cache.clear()
            await mediaCache.clear()
        }
        credentials.clear()
        readReceiptTask?.cancel(); readReceiptTask = nil
        pendingReadReceipts = [:]; viewedReceiptMessageIDs = []
        configuration = nil
        contacts = []
        avatarURLs = [:]
        avatarRequests = []
        messageImages = [:]
        messageMediaURLs = [:]
        rememberedImageSizes = [:]
        galleryDemoMessages = []
        mediaLoadingIDs = []
        mediaErrorIDs = []
        linkPreviews = [:]
        messages = [:]
        reactionEvents = [:]
        topicCatalog = .empty
        topicPages = [:]
        messageTopicStates = [:]
        messageTopicRequests = [:]
        messageTopicFailures = []
        topicAccountEpoch = UUID()
        topicCatalogRequest = UUID()
        topicRevision = UUID()
        topicReloadAfterLoad = []
        topicCatalogError = nil
        feedByID = [:]
        feedCursor = nil
        feedHasMore = false
        feedHasLoaded = false
        feedLoading = false
        feedGeneration = UUID()
        mainTab = .messages
        selectedContactID = nil
        phase = .needsConfiguration
    }

    private func connect(
        configuration: ServerConfiguration,
        remember: Bool,
        restoreCached: Bool = false
    ) async {
        isConnecting = true
        defer { isConnecting = false }
        errorMessage = nil
        readReceiptTask?.cancel(); readReceiptTask = nil
        if self.configuration?.baseURL != configuration.baseURL {
            pendingReadReceipts = [:]; viewedReceiptMessageIDs = []
        }
        self.configuration = configuration
        topicAccountEpoch = UUID()
        await api.configure(configuration)
        var restoredCache = false
        if restoreCached {
            restoredCache = await restoreCachedState()
        }
        phase = restoredCache ? .ready : .connecting
        do {
            applyBackendStatus(try await api.authenticate())
            if remember { try credentials.save(configuration) }
            contacts = try await api.contacts()
            phase = .ready
            startReadReceiptRetry()
            await persistCache()
            await PushNotificationCoordinator.shared.activate(for: self)
            try await api.connectLiveEvents { [weak self] event in
                self?.handle(event)
            }
        } catch {
            presentError("Couldn’t connect", error)
            phase = restoredCache ? .ready : .needsConfiguration
        }
    }

    func restoreCachedState() async -> Bool {
        if !demoMode, rememberedImageSizes.isEmpty {
            rememberedImageSizes = await mediaCache.rememberedImageSizes()
        }
        guard !demoMode,
              let configuration,
              let snapshot = await cache.load(for: configuration) else {
            return false
        }
        contacts = snapshot.contacts
        messages = snapshot.messages
        for (chat, ids) in snapshot.pendingReadReceipts ?? [:] {
            pendingReadReceipts[chat, default: []].formUnion(ids)
            viewedReceiptMessageIDs.formUnion(ids)
        }
        startReadReceiptRetry()
        return true
    }

    func becameActive() async {
        guard phase == .ready, !demoMode, !isConnecting else { return }
        _ = await restoreCachedState()
        await refresh()
        await refreshWhatsAppConnection()
        // Topic assignments may finish while the app is suspended, without a live
        // event reaching this session. Refresh both the catalog and loaded pages.
        await loadTopics()
        if mainTab == .messages { await loadFeed() }
    }

    func registerPushDevice(token: String, installationID: String, environment: String) async {
        guard phase == .ready, !demoMode else { return }
        do {
            try await api.registerPushDevice(
                PushDeviceRegistration(
                    installationId: installationID,
                    token: token,
                    environment: environment
                )
            )
        } catch {
            AppLog.push.error("Push token registration with the server failed: \(AppLog.kind(of: error), privacy: .public) \(error.localizedDescription)")
        }
    }

    func openConversation(fromNotification contactID: String) {
        mainTab = .chats
        selectedContactID = contactID
        Task {
            await refresh()
            selectedContactID = contactID
            await loadMessages(for: contactID)
        }
    }
}
