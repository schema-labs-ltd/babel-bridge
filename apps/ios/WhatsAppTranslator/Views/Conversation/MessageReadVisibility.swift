import SwiftUI

/// A row being created, fetched or translated is not evidence that it was seen.
struct MessageReadVisibility: ViewModifier {
    @Environment(AppSession.self) private var session
    let messages: [ChatMessage]
    let enabled: Bool
    @State private var isVisible = false
    func body(content: Content) -> some View {
        content
            .onScrollVisibilityChange(threshold: 0.5) { isVisible = $0 }
            .modifier(ViewedMessageTask(messages: messages, enabled: enabled && isVisible && session.readReceiptCovers.isEmpty))
    }
}

struct ViewedMessageTask: ViewModifier {
    @Environment(AppSession.self) private var session
    @Environment(\.scenePhase) private var scenePhase
    let messages: [ChatMessage]
    let enabled: Bool
    private var key: String { "\(enabled && scenePhase == .active):" + messages.map(\.id).joined(separator: ",") }
    func body(content: Content) -> some View {
        content.task(id: key) {
            guard enabled, scenePhase == .active else { return }
            do {
                try await Task.sleep(for: .milliseconds(700))
                try Task.checkCancellation()
                await session.recordMessageViews(messages)
            } catch { /* Leaving the viewport, covering the view or suspending cancels the dwell. */ }
        }
    }
}


struct ReadReceiptCover: ViewModifier {
    @Environment(AppSession.self) private var session
    @State private var id = UUID()
    func body(content: Content) -> some View {
        content.onAppear { session.readReceiptCovers.insert(id) }
            .onDisappear { session.readReceiptCovers.remove(id) }
    }
}
