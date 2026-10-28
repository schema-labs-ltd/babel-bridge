import AVKit
import CoreTransferable
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct ComposerView: View {
    @Environment(AppSession.self) private var session
    private var replyBlocked: Bool { !session.canSend(to: contactID, reply: reply) }
    @Environment(\.translatorPalette) private var palette
    let contactID: String
    @Binding var text: String
    let reply: MessageReplyTarget?
    var showReplyPreview = true
    let isSending: Bool
    let cancelReply: () -> Void
    let sendImages: ([OutgoingImage], String?) -> Bool
    let send: () -> Void
    @State private var showVoiceComposer = false
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @State private var pendingPhotos: PendingPhotoSelection?
    @State private var pickerError: String?
    @State private var imagePaste = ImagePasteController()
    @State private var focused = false

    var body: some View {
        VStack(spacing: 0) {
            if replyBlocked {
                Label("Reply only · Choose an incoming message and tap Reply", systemImage: "arrowshape.turn.up.left")
                    .font(.caption).foregroundStyle(.secondary)
                    .padding(.horizontal, 16).padding(.vertical, 8)
                    .accessibilityIdentifier("reply-only-hint")
            }
            if let reply, showReplyPreview {
                HStack(spacing: 10) {
                    RoundedRectangle(cornerRadius: 2)
                        .fill(palette.accent)
                        .frame(width: 3, height: 34)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(reply.senderName)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(palette.deepAccent)
                        Text(reply.text)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer()
                    Button("Cancel reply", systemImage: "xmark.circle.fill", action: cancelReply)
                        .labelStyle(.iconOnly)
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 15)
                .padding(.vertical, 8)
                .translatorGlass(in: RoundedRectangle(cornerRadius: 20))
                .padding(.horizontal, 12)
            }

            ComposerGlassGroup {
                HStack(alignment: .center, spacing: 7) {
                    PhotosPicker(selection: $selectedPhotos, maxSelectionCount: 30, matching: .images) {
                        addImageLabel
                    }
                    .buttonStyle(.plain)
                    .disabled(isSending)
                    .accessibilityLabel("Send photos")
                    .translatorGlassControl(in: Circle())

                    composerInput

                    if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !isSending {
                        Button { showVoiceComposer = true } label: {
                            Image(systemName: "mic.fill")
                                .font(.system(size: 23, weight: .regular))
                                .foregroundStyle(Color.primary)
                                .frame(width: 44, height: 44)
                                .translatorGlassControl(in: Circle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Record voice note")
                        .help("Record voice note")
                    } else {
                        sendButton
                    }
                }
                .disabled(replyBlocked)
                .padding(.horizontal, 12)
                .padding(.top, reply == nil || !showReplyPreview ? 8 : 2)
                .padding(.bottom, 9)
            }
        }
        .modifier(PastedImagePresentation(controller: imagePaste))
        .onChange(of: contactID) { _, _ in imagePaste.cancel() }
        .onChange(of: reply?.messageID) { _, value in
            focused = value != nil
            if imagePaste.isLoading { imagePaste.cancel() }
        }
        .onChange(of: selectedPhotos) { _, items in
            guard !items.isEmpty else { return }
            Task {
                defer { selectedPhotos = [] }
                var prepared: [PendingPhoto] = []
                for item in items {
                    guard let photo = await loadPhoto(item) else {
                        pickerError = "One of the selected photos couldn’t be opened. Please choose the photos again."
                        return
                    }
                    prepared.append(photo)
                }
                pendingPhotos = PendingPhotoSelection(photos: prepared)
            }
        }
        .sheet(isPresented: $showVoiceComposer) {
            VoiceComposerView(contactID: contactID, reply: reply, onSent: cancelReply)
        }
        .sheet(item: $pendingPhotos) { selection in
            ImageComposerSheet(photos: selection.photos, reply: reply, send: sendImages)
        }
        .alert("Couldn’t prepare image", isPresented: pickerErrorPresented) {
            Button("OK") { pickerError = nil }
        } message: {
            Text(pickerError ?? "Please choose another image.")
        }
        .task {
            let arguments = ProcessInfo.processInfo.arguments
            guard arguments.contains("-demoImageComposer")
                    || arguments.contains("-demoImageAlbumComposer")
                    || arguments.contains("-demoOptimizedPhotoComposer"),
                  pendingPhotos == nil else { return }
            let count = arguments.contains("-demoImageAlbumComposer") ? 4 : 1
            let photos = (0..<count).compactMap { index -> PendingPhoto? in
                let image = DemoImageFactory.landscape(size: CGSize(width: 800 - index * 80, height: 600 + index * 40))
                guard let data = image.platformJPEGData(compressionQuality: 0.9) else { return nil }
                return PendingPhoto(data: data, mimeType: "image/jpeg", image: image)
            }
            pendingPhotos = PendingPhotoSelection(photos: photos)
        }
    }

    private var composerInput: some View {
        MessageComposerTextInput(text: $text, focus: $focused, allowsImagePaste: !isSending && !imagePaste.isLoading) { providers in
            guard !isSending, pendingPhotos == nil, !showVoiceComposer else { return }
            focused = false
            imagePaste.begin(providers, reply: reply, send: sendImages)
        }
        .modifier(ComposerInputStyle())
    }

    nonisolated private var addImageLabel: some View {
        Image(systemName: "plus")
            .font(.system(size: 23, weight: .regular))
            .foregroundStyle(Color.primary)
            .frame(width: 44, height: 44)
    }

    @ViewBuilder
    private var sendButton: some View {
        #if os(macOS)
        Button {
            guard !isSending else { return }
            send()
        } label: {
            sendButtonLabel
                .frame(width: 44, height: 44)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.white)
        .frame(width: 44, height: 44)
        .translatorGlassControl(in: Circle(), tint: palette.accent)
        .opacity(isSendDisabled ? 0.42 : 1)
        .disabled(isSendDisabled)
        .allowsHitTesting(!isSending)
        .keyboardShortcut(.return, modifiers: .command)
        .accessibilityLabel(isSending ? "Sending message" : "Send message")
        .accessibilityValue(isSending ? "In progress" : "")
        .animation(.snappy, value: isSending)
        #else
        Button {
            guard !isSending else { return }
            send()
        } label: {
            sendButtonLabel
                .frame(width: 46, height: 46)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(isSendDisabled ? Color.platformSecondaryLabel : .white)
        .translatorGlassControl(in: Circle(), tint: isSendDisabled ? nil : palette.accent)
        .disabled(isSendDisabled)
        .allowsHitTesting(!isSending)
        .accessibilityLabel(isSending ? "Sending message" : "Send message")
        .accessibilityValue(isSending ? "In progress" : "")
        .animation(.snappy, value: isSending)
        #endif
    }

    @ViewBuilder
    private var sendButtonLabel: some View {
        if isSending {
            ProgressView()
                .controlSize(.small)
                .tint(.white)
                .transition(.scale.combined(with: .opacity))
        } else {
            Image(systemName: "paperplane.fill")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(isSendDisabled ? Color.platformSecondaryLabel : .white)
                .transition(.scale.combined(with: .opacity))
        }
    }

    private var isSendDisabled: Bool {
        text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !isSending
    }

    private var pickerErrorPresented: Binding<Bool> {
        Binding(get: { pickerError != nil }, set: { if !$0 { pickerError = nil } })
    }

    private func loadPhoto(_ item: PhotosPickerItem) async -> PendingPhoto? {
        guard let data = try? await item.loadTransferable(type: Data.self),
              let image = PlatformImage(data: data) else {
            return nil
        }
        let mimeType = item.supportedContentTypes.compactMap(\.preferredMIMEType).first ?? "application/octet-stream"
        return PendingPhoto(data: data, mimeType: mimeType, image: image)
    }
}

private struct ComposerInputStyle: ViewModifier {
    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .frame(minHeight: 44)
            .translatorGlassControl(in: RoundedRectangle(cornerRadius: 24, style: .continuous))
    }
}

struct PendingPhoto: Identifiable {
    let id = UUID()
    let data: Data
    let mimeType: String
    let image: PlatformImage
}

struct PendingPhotoSelection: Identifiable {
    let id = UUID()
    let photos: [PendingPhoto]
}

struct PastedImagePresentation: ViewModifier {
    @Bindable var controller: ImagePasteController

    func body(content: Content) -> some View {
        content
            .overlay(alignment: .top) {
                if controller.isLoading {
                    HStack(spacing: 10) {
                        ProgressView().controlSize(.small)
                        Text("Preparing pasted images…").font(.caption)
                        Button("Cancel") { controller.cancel() }
                    }
                    .padding(10).background(.regularMaterial, in: Capsule())
                }
            }
            .sheet(item: $controller.selection) { selection in
                ImageComposerSheet(photos: selection.photos, reply: selection.reply, destination: selection.destination, send: selection.send)
            }
            .alert("Couldn’t paste image", isPresented: Binding(get: { controller.error != nil }, set: { if !$0 { controller.error = nil } })) {
                Button("OK") { controller.error = nil }
            } message: { Text(controller.error ?? "Try copying the image again.") }
            .onDisappear { if controller.isLoading { controller.cancel() } }
    }
}

