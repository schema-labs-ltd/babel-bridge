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
                let watchdog = Task { [weak self] in
                    while !Task.isCancelled {
                        do { try await Task.sleep(for: .seconds(10)) } catch { return }
                        guard let self else { return }
                        await self.checkLiveHealth(generation: generation, socket: socket)
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

    static func reconnectDelay(attempt: Int) -> Double { min(30, pow(2, Double(min(max(attempt, 1), 5)))) }

    private func checkLiveHealth(generation: UUID, socket: URLSessionWebSocketTask) {
        if generation != liveGeneration || Date().timeIntervalSince(lastLiveMessage) > 50 {
            socket.cancel(with: .goingAway, reason: nil)
        }
    }
}
