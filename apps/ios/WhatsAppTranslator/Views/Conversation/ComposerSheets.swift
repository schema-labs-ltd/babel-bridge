import AVKit
import CoreTransferable
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct ImageComposerSheet: View {
    @Environment(\.dismiss) private var dismiss
    let photos: [PendingPhoto]
    let reply: MessageReplyTarget?
    var destination: String? = nil
    let send: ([OutgoingImage], String?) -> Bool
    @State private var caption = ""

    var body: some View {
        NavigationStack {
            VStack(spacing: 16) {
                photoPreview

                if photos.count > 1 {
                    Label("\(photos.count) photos selected", systemImage: "photo.stack.fill")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                }

                if photos.count > 0 {
                    Label("Photos are optimized when you send", systemImage: "wand.and.sparkles")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                }

                if let destination {
                    VStack(spacing: 5) {
                        Text("To: \(destination)").font(.headline)
                        if let reply { Text(reply.text).font(.caption).lineLimit(2).foregroundStyle(.secondary) }
                    }.padding(.horizontal)
                }

                if let reply {
                    Label("Replying to \(reply.senderName)", systemImage: "arrowshape.turn.up.left")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                TextField("Add a caption", text: $caption, axis: .vertical)
                    .lineLimit(1...4)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 16))
                    .padding(.horizontal)

                Spacer(minLength: 0)
            }
            .padding(.top)
            .navigationTitle(photos.count == 1 ? "Send photo" : "Send \(photos.count) photos")
            .platformInlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        let cleanCaption = caption.trimmingCharacters(in: .whitespacesAndNewlines)
                        let images = photos.map { OutgoingImage(data: $0.data, mimeType: $0.mimeType) }
                        if send(images, cleanCaption.isEmpty ? nil : cleanCaption) {
                            dismiss()
                        }
                    } label: {
                        Label("Send", systemImage: "paperplane.fill")
                    }
                    .fontWeight(.semibold)
                }
            }
        }
        #if os(macOS)
        .platformSheetSize(
            minWidth: MacChatLayoutMetrics.mediaSheetMinimumWidth,
            minHeight: MacChatLayoutMetrics.mediaSheetMinimumHeight
        )
        #endif
    }

    @ViewBuilder
    private var photoPreview: some View {
        if photos.count == 1, let photo = photos.first {
            Image(platformImage: photo.image)
                .resizable()
                .scaledToFit()
                .frame(maxWidth: .infinity, maxHeight: 440)
                .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                .padding(.horizontal)
        } else {
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(spacing: 10) {
                    ForEach(Array(photos.enumerated()), id: \.element.id) { index, photo in
                        ZStack(alignment: .topTrailing) {
                            Image(platformImage: photo.image)
                                .resizable()
                                .scaledToFill()
                                .frame(width: 150, height: 210)
                                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                            Text("\(index + 1)")
                                .font(.caption.bold())
                                .foregroundStyle(.white)
                                .frame(width: 26, height: 26)
                                .background(.black.opacity(0.62), in: Circle())
                                .padding(8)
                        }
                    }
                }
                .padding(.horizontal)
            }
            .frame(height: 210)
        }
    }
}

struct ImportedMovie: Transferable {
    let url: URL
    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .movie) { received in
            let copy = FileManager.default.temporaryDirectory.appendingPathComponent("import-\(UUID().uuidString).\(received.file.pathExtension)")
            try FileManager.default.copyItem(at: received.file, to: copy)
            return ImportedMovie(url: copy)
        }
    }
}

final class PendingAttachment: Identifiable {
    let id = UUID()
    let outgoing: OutgoingAttachment
    let previewURL: URL?
    init(outgoing: OutgoingAttachment, previewURL: URL?) {
        self.outgoing = outgoing
        self.previewURL = previewURL
    }
    deinit { if let previewURL { try? FileManager.default.removeItem(at: previewURL) } }

    static func video(from url: URL) async throws -> PendingAttachment {
        let asset = AVURLAsset(url: url)
        guard let exporter = AVAssetExportSession(asset: asset, presetName: AVAssetExportPreset1280x720) else {
            throw APIError.server("This video couldn’t be prepared for WhatsApp.")
        }
        let output = FileManager.default.temporaryDirectory.appendingPathComponent("video-\(UUID().uuidString).mp4")
        do {
            exporter.shouldOptimizeForNetworkUse = true
            try await exporter.export(to: output, as: .mp4)
            let size = try output.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
            guard size > 0, size <= OutgoingAttachment.maximumBytes else {
                throw APIError.server("This video is too large. Choose a shorter clip (up to 64 MB after preparation).")
            }
            let data = try Data(contentsOf: output)
            return PendingAttachment(outgoing: OutgoingAttachment(data: data, mimeType: "video/mp4", fileName: "Video.mp4", kind: "video"), previewURL: output)
        } catch {
            try? FileManager.default.removeItem(at: output)
            throw error
        }
    }
}

struct AttachmentComposerSheet: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let attachment: PendingAttachment
    let context: UnifiedMediaContext
    let onSent: () -> Void
    @State private var caption = ""
    @State private var sending = false
    @State private var attemptedCaption: String?
    @State private var hasAttemptedSend = false
    @State private var failed = false
    @State private var player: AVPlayer?

    var body: some View {
        NavigationStack {
            VStack(spacing: 18) {
                if let player { VideoPlayer(player: player).frame(minHeight: 180, maxHeight: 360) }
                else {
                    Image(systemName: "doc.fill").font(.system(size: 60)).foregroundStyle(.secondary)
                    Text(attachment.outgoing.fileName).font(.headline).lineLimit(3)
                }
                VStack(alignment: .leading, spacing: 5) {
                    Text("To: \(context.destination)").font(.headline)
                    Text(context.reply.text).font(.caption).lineLimit(2).foregroundStyle(.secondary)
                }.frame(maxWidth: .infinity, alignment: .leading)
                TextField("Add a caption", text: $caption, axis: .vertical)
                    .lineLimit(1...4).textFieldStyle(.roundedBorder).disabled(sending || hasAttemptedSend)
                if sending { ProgressView("Sending attachment…") }
                if failed { Text("The attachment wasn’t confirmed. Try again to check its delivery safely.").font(.caption).foregroundStyle(.red) }
                Spacer(minLength: 0)
            }
            .padding()
            .navigationTitle(attachment.outgoing.kind == "video" ? "Send video" : "Send file")
            .platformInlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(sending) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Send", systemImage: "paperplane.fill") {
                        guard !sending else { return }
                        sending = true; failed = false; player?.pause()
                        if !hasAttemptedSend {
                            let clean = caption.trimmingCharacters(in: .whitespacesAndNewlines)
                            attemptedCaption = clean.isEmpty ? nil : clean
                            hasAttemptedSend = true
                        }
                        Task {
                            let sent = await session.sendAttachment(attachment.outgoing, caption: attemptedCaption, to: context.message.contactId, reply: context.reply, replyOnlyIfNotLatest: true)
                            sending = false
                            if sent { onSent(); dismiss() } else { failed = true }
                        }
                    }.disabled(sending)
                }
            }
        }
        .interactiveDismissDisabled(sending)
        .onAppear { if let url = attachment.previewURL { player = AVPlayer(url: url) } }
        .onDisappear { player?.pause(); player = nil }
        #if os(macOS)
        .frame(minWidth: 480, minHeight: 480)
        #endif
    }
}
