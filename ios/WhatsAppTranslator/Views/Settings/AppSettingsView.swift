import AuthenticationServices
import Network
import SwiftUI

struct AppSettingsView: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    @State private var settings = OpenAISettings(model: nil, reasoningEffort: nil, available: nil)
    @State private var isLoading = true
    @State private var isSaving = false
    @State private var error: String?

    private let models = [
        ("", "App defaults"),
        ("gpt-6-astra", "GPT-6 Astra"),
        ("gpt-5.6-sol", "GPT-5.6 Sol · most capable"),
        ("gpt-5.6-terra", "GPT-5.6 Terra · balanced"),
        ("gpt-5.6-luna", "GPT-5.6 Luna · fastest value"),
    ]
    private let efforts = [
        ("", "Task default"), ("none", "None"), ("low", "Low"),
        ("medium", "Medium"), ("high", "High"), ("xhigh", "Extra high"), ("max", "Maximum"),
    ]

    var body: some View {
        NavigationStack {
            Form {
                Section("Notifications") { MessageToneSettingsRow() }
                VoicePreferenceSection(scope: "outgoing")

                Section("Connection") {
                    LabeledContent("Server", value: session.configuration?.baseURL.host() ?? "Not configured")
                    LabeledContent("WhatsApp", value: session.backendStatus.connected ? "Connected" : "Disconnected")
                }

                Section {
                    if isLoading {
                        HStack { Spacer(); ProgressView("Loading GPT settings…"); Spacer() }
                    }
                    Picker("GPT model", selection: modelBinding) {
                        ForEach(models, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    .platformCompactControlTypography()
                    .disabled(isLoading || isSaving)
                    Picker("Reasoning", selection: effortBinding) {
                        ForEach(efforts, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    .platformCompactControlTypography()
                    .disabled(isLoading || isSaving)
                } header: {
                    Text("OpenAI")
                } footer: {
                    Text("App defaults preserve the backend’s task-specific model and reasoning choices.")
                }

                Section {
                    ChatGPTAccountRows()
                } header: {
                    Text("ChatGPT account")
                } footer: {
                    Text("Signing in uses your ChatGPT plan for translation, AI replies and topics instead of the server’s API key.")
                }

                Section {
                    NavigationLink {
                        AppIconPickerView()
                    } label: {
                        LabeledContent("App icon", value: session.appIcons.selected.title)
                    }
                    .accessibilityIdentifier("app-icon-settings")
                    Picker("Theme", selection: themeBinding) {
                        ForEach(AppTheme.allCases) { theme in Text(theme.title).tag(theme) }
                    }
                    .platformCompactControlTypography()
                    Picker("Appearance", selection: colorModeBinding) {
                        ForEach(AppColorMode.allCases) { mode in Text(mode.title).tag(mode) }
                    }
                    .platformCompactControlTypography()
                    NavigationLink {
                        WallpaperPickerView()
                    } label: {
                        LabeledContent("Chat wallpaper", value: session.preferences.wallpaper.title)
                    }
                    ThemePreview(theme: session.preferences.theme)
                } header: {
                    Text("Appearance")
                } footer: {
                    Text("Themes change the native chat palette. Appearance can follow the system or stay light or dark.")
                }

                Section {
                    Button("Change server", role: .destructive) {
                        dismiss()
                        session.forgetServer()
                    }
                } footer: {
                    Text("This forgets the address and password on this device. It does not log WhatsApp out or delete backend data.")
                }
            }
            .navigationTitle("Settings")
            .platformInlineNavigationTitle()
            .toolbar {
                #if os(macOS)
                ToolbarItem(placement: .primaryAction) {
                    Button("Save GPT") { save() }
                        .fontWeight(.semibold)
                        .disabled(isLoading || isSaving)
                }
                #else
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save GPT") { save() }
                        .fontWeight(.semibold)
                        .disabled(isLoading || isSaving)
                }
                #endif
            }
            .task { await load() }
            .alert("Couldn’t save settings", isPresented: errorPresented) {
                Button("OK") { error = nil }
            } message: { Text(error ?? "Please try again.") }
        }
        #if os(macOS)
        .platformSheetSize(
            minWidth: MacChatLayoutMetrics.settingsSheetMinimumWidth,
            minHeight: MacChatLayoutMetrics.settingsSheetMinimumHeight
        )
        #endif
    }

    private var modelBinding: Binding<String> {
        Binding(get: { settings.model ?? "" }, set: { settings.model = $0.isEmpty ? nil : $0 })
    }

    private var effortBinding: Binding<String> {
        Binding(get: { settings.reasoningEffort ?? "" }, set: { settings.reasoningEffort = $0.isEmpty ? nil : $0 })
    }

    private var themeBinding: Binding<AppTheme> {
        Binding(get: { session.preferences.theme }, set: { session.preferences.theme = $0 })
    }

    private var colorModeBinding: Binding<AppColorMode> {
        Binding(get: { session.preferences.colorMode }, set: { session.preferences.colorMode = $0 })
    }

    private var errorPresented: Binding<Bool> {
        Binding(get: { error != nil }, set: { if !$0 { error = nil } })
    }

    private func load() async {
        do { settings = try await session.openAISettings() }
        catch { self.error = error.localizedDescription }
        isLoading = false
    }

    private func save() {
        isSaving = true
        Task {
            do {
                try await session.saveOpenAISettings(settings)
                dismiss()
            } catch {
                self.error = error.localizedDescription
                isSaving = false
            }
        }
    }
}

private struct ThemePreview: View {
    let theme: AppTheme

    var body: some View {
        let palette = TranslatorPalette.make(theme)
        VStack(spacing: 8) {
            HStack {
                Text("Incoming message")
                    .padding(9)
                    .background(palette.incomingBubble, in: RoundedRectangle(cornerRadius: 12))
                Spacer()
            }
            HStack {
                Spacer()
                Text("Translated reply")
                    .padding(9)
                    .background(palette.outgoingBubble, in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .font(.caption)
        .padding(12)
        .background(palette.chatBackground, in: RoundedRectangle(cornerRadius: 14))
        .platformCompactControlTypography()
        .animation(.snappy, value: theme)
    }
}

private struct WallpaperPickerView: View {
    @Environment(AppSession.self) private var session
    @Environment(\.colorScheme) private var colorScheme
    @State private var previewDark = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Picker("Preview appearance", selection: $previewDark) {
                    Text("Light").tag(false)
                    Text("Dark").tag(true)
                }
                .pickerStyle(.segmented)
                Text("Choose a background for all chats. Each design automatically adapts to your appearance.")
                    .font(.subheadline).foregroundStyle(.secondary)
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 145), spacing: 14)], spacing: 14) {
                    ForEach(AppWallpaper.allCases) { wallpaper in
                        Button {
                            session.preferences.wallpaper = wallpaper
                        } label: {
                            VStack(alignment: .leading, spacing: 0) {
                                TranslatorPalette.make(session.preferences.theme).chatBackground
                                    .overlay { WallpaperPattern(wallpaper: wallpaper, tileSize: 180) }
                                    .environment(\.colorScheme, previewDark ? .dark : .light)
                                    .frame(height: 156)
                                HStack {
                                    Text(wallpaper.title).font(.subheadline.weight(.medium))
                                    Spacer(minLength: 4)
                                    Image(systemName: session.preferences.wallpaper == wallpaper ? "checkmark.circle.fill" : "circle")
                                        .foregroundStyle(session.preferences.wallpaper == wallpaper ? Color.accentColor : Color.secondary)
                                }
                                .padding(12)
                            }
                            .background(.background)
                            .clipShape(RoundedRectangle(cornerRadius: 16))
                            .overlay {
                                RoundedRectangle(cornerRadius: 16)
                                    .stroke(session.preferences.wallpaper == wallpaper ? Color.accentColor : Color.secondary.opacity(0.2), lineWidth: session.preferences.wallpaper == wallpaper ? 2 : 1)
                            }
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(wallpaper.title)
                        .accessibilityAddTraits(session.preferences.wallpaper == wallpaper ? .isSelected : [])
                    }
                }
            }
            .padding(20)
        }
        .navigationTitle("Chat wallpaper")
        .platformInlineNavigationTitle()
        .onAppear { previewDark = colorScheme == .dark }
    }
}

/// Sign in with ChatGPT controls, shared by the iOS and macOS settings screens.
struct ChatGPTAccountRows: View {
    @Environment(AppSession.self) private var session
    @State private var status: ChatGPTStatus?
    @State private var isBusy = false
    @State private var message: String?
    @State private var coordinator = ChatGPTSignInCoordinator()

    var body: some View {
        Group {
            if let status, status.available == false {
                Text("This server doesn’t offer Sign in with ChatGPT.")
                    .foregroundStyle(.secondary)
            } else if let status, status.signedIn {
                Text(signedInText(status))
                    .font(.callout)
                Picker("ChatGPT model", selection: modelBinding) {
                    ForEach(status.models ?? []) { Text($0.displayName).tag($0.slug) }
                }
                .platformCompactControlTypography()
                .disabled(isBusy || (status.models ?? []).isEmpty)
                Button("Sign out of ChatGPT", role: .destructive) { signOut() }
                    .disabled(isBusy)
            } else {
                Text(status?.apiKeyConfigured == true
                    ? "Not signed in. Text AI uses the server’s OpenAI API key."
                    : "Not signed in. Sign in to use your ChatGPT plan for translation and AI replies.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Button {
                    signIn()
                } label: {
                    if isBusy { ProgressView() } else { Text("Sign in with ChatGPT") }
                }
                .disabled(isBusy || status == nil)
                .accessibilityIdentifier("chatgpt-sign-in")
            }
            if let message {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .task { await load() }
    }

    private func signedInText(_ status: ChatGPTStatus) -> String {
        let who = status.email ?? status.name ?? "your ChatGPT account"
        let voice = status.apiKeyConfigured == true
            ? "Voice notes still use the server’s API key."
            : "Voice notes need an OpenAI API key on the server."
        return "Signed in as \(who). Usage counts toward your ChatGPT plan limits. \(voice)"
    }

    private var modelBinding: Binding<String> {
        Binding(
            get: { status?.model ?? "" },
            set: { model in
                guard !model.isEmpty, model != status?.model else { return }
                run { try await session.setChatGPTModel(model) }
            }
        )
    }

    private func load() async {
        do { status = try await session.chatGPTStatus() } catch { message = error.localizedDescription }
    }

    private func signIn() {
        run { try await coordinator.signIn(session: session) }
    }

    private func signOut() {
        run {
            let result = try await session.signOutOfChatGPT()
            if result.revocationConfirmed == false {
                message = "Signed out here, but ChatGPT didn’t confirm it. You can disconnect Babel Bridge in ChatGPT Settings."
            }
            return result
        }
    }

    private func run(_ action: @escaping @MainActor () async throws -> ChatGPTStatus) {
        isBusy = true
        message = nil
        Task {
            do {
                status = try await action()
            } catch ChatGPTSignInError.cancelled {
                // The person closed the sign-in sheet.
            } catch {
                message = error.localizedDescription
            }
            isBusy = false
        }
    }
}

enum ChatGPTSignInError: LocalizedError {
    case cancelled
    case couldNotStart

    var errorDescription: String? {
        switch self {
        case .cancelled: "Sign-in was cancelled."
        case .couldNotStart: "Couldn’t open ChatGPT sign-in. Please try again."
        }
    }
}

/// Runs Sign in with ChatGPT on this device. ChatGPT redirects to 127.0.0.1, so a
/// short-lived loopback listener catches the callback and passes it to the server,
/// which holds the PKCE secret, exchanges the code and keeps the tokens.
@MainActor
final class ChatGPTSignInCoordinator: NSObject, ASWebAuthenticationPresentationContextProviding {
    private var listener: NWListener?
    private var authSession: ASWebAuthenticationSession?
    private var portContinuation: CheckedContinuation<UInt16, Error>?
    private var callbackContinuation: CheckedContinuation<String, Error>?
    private var port: UInt16 = 0

    func signIn(session: AppSession) async throws -> ChatGPTStatus {
        stop()
        defer { stop() }
        port = try await startListener()
        let start = try await session.startChatGPTSignIn(
            redirectURI: "http://127.0.0.1:\(port)/auth/callback"
        )
        guard let url = URL(string: start.authorizeUrl) else { throw ChatGPTSignInError.couldNotStart }
        let callback = try await withCheckedThrowingContinuation { continuation in
            callbackContinuation = continuation
            // The callback scheme never fires: the loopback listener receives the redirect.
            let auth = ASWebAuthenticationSession(url: url, callbackURLScheme: "babelbridge-chatgpt") { [weak self] _, _ in
                Task { @MainActor in self?.finish(.failure(ChatGPTSignInError.cancelled)) }
            }
            auth.presentationContextProvider = self
            authSession = auth
            if !auth.start() { finish(.failure(ChatGPTSignInError.couldNotStart)) }
        }
        return try await session.completeChatGPTSignIn(callbackURL: callback)
    }

    private func startListener() async throws -> UInt16 {
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: .any)
        parameters.acceptLocalOnly = true
        let listener = try NWListener(using: parameters)
        self.listener = listener
        listener.newConnectionHandler = { [weak self] connection in
            Task { @MainActor in self?.handle(connection) }
        }
        return try await withCheckedThrowingContinuation { continuation in
            portContinuation = continuation
            listener.stateUpdateHandler = { [weak self] state in
                Task { @MainActor in self?.listenerChanged(state) }
            }
            listener.start(queue: .main)
        }
    }

    private func listenerChanged(_ state: NWListener.State) {
        switch state {
        case .ready:
            if let port = listener?.port?.rawValue {
                portContinuation?.resume(returning: port)
                portContinuation = nil
            }
        case .failed(let error):
            portContinuation?.resume(throwing: error)
            portContinuation = nil
            finish(.failure(error))
        default:
            break
        }
    }

    private func handle(_ connection: NWConnection) {
        connection.start(queue: .main)
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65_536) { [weak self] data, _, _, _ in
            let target = Self.requestTarget(data.flatMap { String(data: $0, encoding: .utf8) } ?? "")
            let isCallback = target?.hasPrefix("/auth/callback?") == true
            let body = isCallback
                ? "<!doctype html><meta charset=utf-8><title>Babel Bridge</title><p style=\"font:16px -apple-system,sans-serif;padding:2em\">You’re signed in. You can return to Babel Bridge.</p>"
                : "Not found"
            let response = "HTTP/1.1 \(isCallback ? "200 OK" : "404 Not Found")\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: \(body.utf8.count)\r\nConnection: close\r\n\r\n\(body)"
            connection.send(content: Data(response.utf8), completion: .contentProcessed { _ in connection.cancel() })
            if isCallback, let target {
                Task { @MainActor in self?.receivedCallback(target) }
            }
        }
    }

    /// The path and query from an HTTP request line such as `GET /auth/callback?code=… HTTP/1.1`.
    nonisolated static func requestTarget(_ request: String) -> String? {
        let parts = request.split(separator: "\r\n", maxSplits: 1).first?.split(separator: " ") ?? []
        guard parts.count >= 2, parts[0] == "GET" else { return nil }
        return String(parts[1])
    }

    private func receivedCallback(_ target: String) {
        finish(.success("http://127.0.0.1:\(port)\(target)"))
    }

    private func finish(_ result: Result<String, Error>) {
        guard let continuation = callbackContinuation else { return }
        callbackContinuation = nil
        continuation.resume(with: result)
        authSession?.cancel()
    }

    private func stop() {
        listener?.cancel()
        listener = nil
        authSession?.cancel()
        authSession = nil
        portContinuation?.resume(throwing: ChatGPTSignInError.cancelled)
        portContinuation = nil
        finish(.failure(ChatGPTSignInError.cancelled))
    }

    nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            #if os(macOS)
            NSApplication.shared.keyWindow ?? NSApplication.shared.windows.first ?? ASPresentationAnchor()
            #else
            let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            if let window = scenes.flatMap(\.windows).first(where: \.isKeyWindow) { return window }
            if let scene = scenes.first { return UIWindow(windowScene: scene) }
            return ASPresentationAnchor()
            #endif
        }
    }
}
