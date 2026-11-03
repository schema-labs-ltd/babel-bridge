import SwiftUI

struct ContactAvatar: View {
    @Environment(\.translatorPalette) private var palette
    let contact: Contact
    let url: URL?
    var size: CGFloat = 52
    /// A picture saved on this device. `AsyncImage` loads only web addresses, so saved files
    /// are read here instead.
    @State private var savedImage: PlatformImage?

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            Group {
                if let url, url.isFileURL {
                    if let savedImage {
                        Image(platformImage: savedImage)
                            .resizable()
                            .scaledToFill()
                    } else {
                        placeholder
                    }
                } else {
                    AsyncImage(url: url) { phase in
                        switch phase {
                        case .success(let image):
                            image
                                .resizable()
                                .scaledToFill()
                        default:
                            placeholder
                        }
                    }
                }
            }
            .frame(width: size, height: size)
            .clipShape(Circle())

            if contact.isGroup {
                Image(systemName: "person.2.fill")
                    .font(.system(size: max(7, size * 0.15), weight: .bold))
                    .foregroundStyle(.white)
                    .padding(max(4, size * 0.09))
                    .background(palette.accent, in: Circle())
            }
        }
        .frame(width: size, height: size)
        .task(id: url) {
            guard let url, url.isFileURL else {
                savedImage = nil
                return
            }
            let data = await Task.detached(priority: .userInitiated) { try? Data(contentsOf: url) }.value
            guard !Task.isCancelled else { return }
            savedImage = data.flatMap { PlatformImage(data: $0) }
        }
    }

    private var placeholder: some View {
        Circle()
            .fill(contact.isGroup ? Color.indigo.opacity(0.16) : palette.accent.opacity(0.14))
            .overlay {
                Text(contact.initials)
                    .font(.system(size: size * 0.34, weight: .semibold))
                    .foregroundStyle(contact.isGroup ? .indigo : palette.deepAccent)
            }
    }
}
