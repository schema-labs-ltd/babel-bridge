import AVKit
import CoreTransferable
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// Who an attachment goes to. It is captured when you pick an option, before any picker or
/// recorder appears, and never inferred again at send time.
struct AttachmentContext {
    let contactID: String
    let reply: MessageReplyTarget?
    var destination: String? = nil
    var replyOnlyIfNotLatest = false
    /// Sends chosen or captured photos with their caption, and reports whether the send started.
    let sendImages: ([OutgoingImage], String?) -> Bool
    /// Runs after a voice note, video or file has been sent.
    let onSent: () -> Void
}

/// The composer's "+" button and its menu of photos, camera, video, file and voice note, shared by
/// a single chat and the unified Messages feed so both offer the same choices in the same style.
struct AttachmentMenuButton: View {
    let disabled: Bool
    /// Heads the menu, such as "Send to Ana", when the destination isn't obvious from the screen.
    var title: String? = nil
    var height: CGFloat = 44
    var accessibilityIdentifier = "attachment-menu"
    let begin: () -> AttachmentContext?
    @State private var context: AttachmentContext?
    @State private var photosPresented = false
    @State private var cameraPresented = false
    @State private var videoPresented = false
    @State private var filesPresented = false
    @State private var voicePresented = false
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @State private var selectedVideo: PhotosPickerItem?
    @State private var pendingPhotos: PendingPhotoSelection?
    @State private var pendingAttachment: PendingAttachment?
    @State private var loading = false
    @State private var error: String?

    var body: some View {
        Menu {
            if let title {
                Section(title) { menuItems }
            } else {
                menuItems
            }
        } label: {
            Group {
                if loading { ProgressView().controlSize(.small) }
                else { Image(systemName: "plus").font(.system(size: 23, weight: .regular)) }
            }
            .foregroundStyle(Color.primary)
            .frame(width: 44, height: height)
            .contentShape(Circle())
            .translatorGlassControl(in: Circle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        #if os(macOS)
        .menuIndicator(.hidden)
        #endif
        // Keep Photos first, as written, rather than reversing the list when it opens upwards.
        .menuOrder(.fixed)
        .accessibilityLabel("Add attachment")
        .accessibilityIdentifier(accessibilityIdentifier)
        .help("Send photos, videos, files, or a voice note")
        .disabled(disabled || loading)
        .photosPicker(isPresented: $photosPresented, selection: $selectedPhotos, maxSelectionCount: 30, matching: .images)
        .cameraCapture(isPresented: $cameraPresented) { photo in
            guard context != nil else { return }
            pendingPhotos = PendingPhotoSelection(photos: [photo])
        } onFailure: {
            error = "The photo couldn’t be captured. Please try again."
        }
        .photosPicker(isPresented: $videoPresented, selection: $selectedVideo, matching: .videos)
        .fileImporter(isPresented: $filesPresented, allowedContentTypes: [.item]) { result in
            switch result {
            case .success(let url):
                prepareFile(url)
            case .failure(let failure): error = failure.localizedDescription
            }
        }
        .onChange(of: selectedPhotos) { _, items in
            guard !items.isEmpty, context != nil else { return }
            loading = true
            Task {
                defer { loading = false; selectedPhotos = [] }
                var photos: [PendingPhoto] = []
                for item in items {
                    guard let data = try? await item.loadTransferable(type: Data.self),
                          let image = PlatformImage(data: data) else {
                        error = "A photo couldn’t be opened. Please choose it again."
                        return
                    }
                    photos.append(PendingPhoto(data: data, mimeType: item.supportedContentTypes.compactMap(\.preferredMIMEType).first ?? "image/jpeg", image: image))
                }
                pendingPhotos = PendingPhotoSelection(photos: photos)
            }
        }
        .onChange(of: selectedVideo) { _, item in
            guard let item, context != nil else { return }
            loading = true
            Task {
                defer { loading = false; selectedVideo = nil }
                do {
                    guard let movie = try await item.loadTransferable(type: ImportedMovie.self) else {
                        throw APIError.server("The video couldn’t be opened.")
                    }
                    defer { try? FileManager.default.removeItem(at: movie.url) }
                    pendingAttachment = try await PendingAttachment.video(from: movie.url)
                } catch { self.error = error.localizedDescription }
            }
        }
        .sheet(item: $pendingPhotos) { selection in
            if let context {
                ImageComposerSheet(photos: selection.photos, reply: context.reply, destination: context.destination, send: context.sendImages)
                    .enabledInsideDisabledComposer()
            }
        }
        .sheet(item: $pendingAttachment, onDismiss: { pendingAttachment = nil }) { attachment in
            if let context {
                AttachmentComposerSheet(attachment: attachment, context: context)
                    .enabledInsideDisabledComposer()
            }
        }
        .sheet(isPresented: $voicePresented) {
            if let context {
                VoiceComposerView(contactID: context.contactID, reply: context.reply, replyOnlyIfNotLatest: context.replyOnlyIfNotLatest, destination: context.destination, onSent: context.onSent)
                    .enabledInsideDisabledComposer()
            }
        }
        .alert("Couldn’t prepare attachment", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK") { error = nil }
        } message: { Text(error ?? "Please choose another file.") }
    }

    @ViewBuilder
    private var menuItems: some View {
        Button("Photos", systemImage: "photo.on.rectangle") { start { photosPresented = true } }
        if CameraCapture.isAvailable {
            Button("Camera", systemImage: "camera") { start { openCamera() } }
        }
        Button("Video", systemImage: "video") { start { videoPresented = true } }
        Button("File", systemImage: "doc") { start { filesPresented = true } }
        Button("Voice note", systemImage: "mic") { start { voicePresented = true } }
    }

    private func start(_ present: () -> Void) {
        guard let captured = begin() else { return }
        context = captured
        present()
    }

    private func openCamera() {
        Task {
            if await CameraCapture.requestAccess() {
                cameraPresented = true
            } else {
                error = CameraCapture.accessDeniedMessage
            }
        }
    }

    private func prepareFile(_ url: URL) {
        guard context != nil else { return }
        loading = true
        Task {
            defer { loading = false }
            let accessible = url.startAccessingSecurityScopedResource()
            defer { if accessible { url.stopAccessingSecurityScopedResource() } }
            do {
                let type = try url.resourceValues(forKeys: [.contentTypeKey]).contentType
                    ?? UTType(filenameExtension: url.pathExtension) ?? .data
                if type.conforms(to: .movie) {
                    pendingAttachment = try await PendingAttachment.video(from: url)
                } else {
                    let outgoing = try await Task.detached(priority: .userInitiated) {
                        let values = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
                        guard values.isRegularFile == true, let size = values.fileSize, size > 0, size <= OutgoingAttachment.maximumBytes else {
                            throw APIError.server("Choose a file smaller than 64 MB.")
                        }
                        return OutgoingAttachment(data: try Data(contentsOf: url), mimeType: type.preferredMIMEType ?? "application/octet-stream", fileName: url.lastPathComponent, kind: "document")
                    }.value
                    pendingAttachment = PendingAttachment(outgoing: outgoing, previewURL: nil)
                }
            } catch { self.error = error.localizedDescription }
        }
    }
}

private extension View {
    /// The menu sits inside a composer that disables itself while a send runs or when a reply-only
    /// chat has no reply chosen, and sheets inherit that. A sheet that's already open stays usable.
    func enabledInsideDisabledComposer() -> some View {
        environment(\.isEnabled, true)
    }
}
