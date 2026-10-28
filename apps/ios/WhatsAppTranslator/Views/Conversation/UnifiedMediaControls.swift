import AVKit
import CoreTransferable
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// Captured before presenting any picker or recorder, never inferred again at send time.
struct UnifiedMediaContext {
    let message: ChatMessage
    let reply: MessageReplyTarget
    let destination: String
}

struct UnifiedMediaControls: View {
    @Environment(AppSession.self) private var session
    let disabled: Bool
    let begin: () -> UnifiedMediaContext?
    let onSent: (ChatMessage) -> Void
    @State private var context: UnifiedMediaContext?
    @State private var choices = false
    @State private var photosPresented = false
    @State private var videoPresented = false
    @State private var filesPresented = false
    @State private var voicePresented = false
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @State private var selectedVideo: PhotosPickerItem?
    @State private var pendingPhotos: PendingPhotoSelection?
    @State private var pendingAttachment: PendingAttachment?
    @State private var loading = false
    @State private var error: String?

    private var unavailable: Bool {
        disabled || loading
    }

    var body: some View {
        Button {
            guard let captured = begin() else { return }
            context = captured
            choices = true
        } label: {
            Group {
                if loading { ProgressView().controlSize(.small) }
                else { Image(systemName: "plus").font(.system(size: 23)) }
            }.frame(width: 44, height: 46)
                .translatorGlassControl(in: Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Add attachment")
        .help("Send photos, videos, files, or a voice note")
        .disabled(unavailable)
        #if os(macOS)
        .popover(isPresented: $choices, arrowEdge: .bottom) {
            VStack(alignment: .leading, spacing: 12) {
                Text(context.map { "Send to \($0.destination)" } ?? "Add attachment").font(.headline)
                Button("Photos", systemImage: "photo.on.rectangle") { choices = false; photosPresented = true }
                Button("Video", systemImage: "video") { choices = false; videoPresented = true }
                Button("File", systemImage: "doc") { choices = false; filesPresented = true }
                Button("Voice note", systemImage: "mic") { choices = false; voicePresented = true }
            }
            .buttonStyle(.borderless)
            .padding(18)
            .frame(minWidth: 210, alignment: .leading)
        }
        #else
        .confirmationDialog(context.map { "Send to \($0.destination)" } ?? "Add attachment", isPresented: $choices, titleVisibility: .visible) {
            Button("Photos") { photosPresented = true }
            Button("Video") { videoPresented = true }
            Button("File") { filesPresented = true }
            Button("Voice note") { voicePresented = true }
            Button("Cancel", role: .cancel) {}
        }
        #endif
        .photosPicker(isPresented: $photosPresented, selection: $selectedPhotos, maxSelectionCount: 30, matching: .images)
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
                ImageComposerSheet(photos: selection.photos, reply: context.reply, destination: context.destination) { images, caption in
                    if session.startPhotoSend(images, caption: caption, to: context.message.contactId, reply: context.reply, replyOnlyIfNotLatest: true) {
                        onSent(context.message)
                        return true
                    }
                    return false
                }
            }
        }
        .sheet(item: $pendingAttachment, onDismiss: { pendingAttachment = nil }) { attachment in
            if let context {
                AttachmentComposerSheet(attachment: attachment, context: context) { onSent(context.message) }
            }
        }
        .sheet(isPresented: $voicePresented) {
            if let context {
                VoiceComposerView(contactID: context.message.contactId, reply: context.reply, replyOnlyIfNotLatest: true, destination: context.destination) { onSent(context.message) }
            }
        }
        .alert("Couldn’t prepare attachment", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK") { error = nil }
        } message: { Text(error ?? "Please choose another file.") }
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
