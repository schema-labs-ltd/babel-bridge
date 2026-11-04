import Foundation

extension APIClient {
    func connectLiveEvents(handler: @escaping @MainActor @Sendable (LiveEvent) -> Void) throws {
        guard configuration != nil else { throw APIError.notConfigured }
        disconnectLiveEvents()
        let generation = liveGeneration
        receiveTask = Task { [weak self] in
            guard let self else { return }
            await self.runLiveConnection(generation: generation, handler: handler)
        }
    }

    func disconnectLiveEvents() {
        liveGeneration = UUID()
        receiveTask?.cancel()
        receiveTask = nil
        webSocket?.cancel(with: .goingAway, reason: nil)
        webSocket = nil
    }

    private func runLiveConnection(
        generation: UUID,
        handler: @escaping @MainActor @Sendable (LiveEvent) -> Void
    ) async {
        var retry = 0
        while !Task.isCancelled && generation == liveGeneration {
            do {
                guard let configuration else { return }
                if retry > 0 { let _: BackendStatus = try await authorizedRequest("/api/status") }
                guard generation == liveGeneration && !Task.isCancelled else { return }
                guard let url = configuration.webSocketURL(token: token) else { throw APIError.invalidServer }
                let socket = session.webSocketTask(with: url)
                webSocket = socket
                lastLiveMessage = Date()
                socket.resume()
                sendLivePresence(on: socket)
                let watchdog = Task { [weak self] in
                    var ticks = 0
                    while !Task.isCancelled {
                        do { try await Task.sleep(for: .seconds(10)) } catch { return }
                        guard let self else { return }
                        await self.checkLiveHealth(generation: generation, socket: socket)
                        ticks += 1
                        // The server stops counting an app that hasn't said so in 75 seconds.
                        if ticks % 3 == 0 { await self.sendLivePresence(on: socket) }
                    }
                }
                defer { watchdog.cancel(); socket.cancel(with: .goingAway, reason: nil) }
                var receivedStatus = false
                while !Task.isCancelled && generation == liveGeneration {
                    let message = try await socket.receive()
                    lastLiveMessage = Date()
                    let data: Data
                    switch message {
                    case .string(let value): data = Data(value.utf8)
                    case .data(let value): data = value
                    @unknown default: continue
                    }
                    if let event = try? JSONDecoder.backend.decode(LiveEvent.self, from: data) {
                        if event.type == "status" && !receivedStatus {
                            receivedStatus = true
                            retry = 0
                            await handler(.signal("live_restored"))
                        }
                        await handler(event)
                    }
                }
            } catch {
                guard !Task.isCancelled && generation == liveGeneration else { return }
                await handler(.signal("live_reconnecting"))
                retry += 1
                do { try await Task.sleep(for: .seconds(Self.reconnectDelay(attempt: retry))) } catch { return }
            }
        }
    }

    /// The first retry is almost immediate: it is usually the socket iOS closed while the app was
    /// suspended, and the server is fine. Later retries back off to 2, 4, 8, 16 and then 30 seconds.
    static func reconnectDelay(attempt: Int) -> Double {
        attempt <= 1 ? 0.25 : min(30, pow(2, Double(min(attempt - 1, 5))))
    }

    /// While the app is in front of the user, the server keeps the account online, as WhatsApp
    /// Web does, so WhatsApp sends typing indicators.
    func setLiveForeground(_ foreground: Bool) {
        guard liveForeground != foreground else { return }
        liveForeground = foreground
        if let webSocket { sendLivePresence(on: webSocket) }
    }

    private func sendLivePresence(on socket: URLSessionWebSocketTask) {
        let message = #"{"type":"presence","active":\#(liveForeground)}"#
        socket.send(.string(message)) { _ in }
    }

    private func checkLiveHealth(generation: UUID, socket: URLSessionWebSocketTask) {
        if generation != liveGeneration || Date().timeIntervalSince(lastLiveMessage) > 50 {
            socket.cancel(with: .goingAway, reason: nil)
        }
    }
}
