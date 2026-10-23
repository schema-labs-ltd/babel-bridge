import SwiftUI

// iPhone Duo support for SwiftUI surfaces. The layout rules themselves live in FoldableLayoutModel.swift.
//
// The device has a compact outer display and a regular-width inner display with a hinge that
// becomes an active reserved "division" region while the device is partially open. The system
// also moves toolbars and tab bars to the side of the display in several poses. Everything that
// depends on the iOS 27 or 27.1 SDK is compiled only with Swift 6.4 (Xcode 27.x) and resolved at
// runtime with availability checks, so Xcode 26.6 continuous integration keeps building the same
// sources without any of these features.

extension FoldAwareChatLayout.GridPlan {
    var gridItems: [GridItem] {
        (0..<columnCount).map { index in
            GridItem(.flexible(), spacing: index == gutterAfterColumn ? gutterWidth : Self.tileSpacing)
        }
    }
}

// MARK: - Reading the fold and the bars

/// The layout facts a surface needs to stay clear of the hinge and to follow vertical bars.
struct FoldableLayoutContext: Equatable {
    var size: CGSize = .zero
    var safeAreaInsets: EdgeInsets = EdgeInsets()
    /// The active fold overlapping the surface, or `nil` on a flat display.
    var division: FoldDivision? = nil
    /// The edge the system reserves for vertical bars, or `nil` where bars stay horizontal.
    var verticalBarEdge: HorizontalEdge? = nil

    var usesVerticalBars: Bool { verticalBarEdge != nil }
}

extension GeometryProxy {
    /// The active fold overlapping this view on iPhone Duo; `nil` on flat displays and earlier systems.
    var activeFoldDivision: FoldDivision? {
        #if os(iOS) && compiler(>=6.4)
        if #available(iOS 27.1, *) {
            guard let region = reservedRegions(kind: .division).first(where: { $0.isActive }) else { return nil }
            return FoldDivision(frame: region.frame)
        }
        #endif
        return nil
    }
}

/// Reads the fold and the bar placement for its content, like `GeometryReader` for a whole surface.
struct FoldableLayoutReader<Content: View>: View {
    @ViewBuilder var content: (FoldableLayoutContext) -> Content

    var body: some View {
        GeometryReader { proxy in
            FoldableBarReader { edge in
                content(
                    FoldableLayoutContext(
                        size: proxy.size,
                        safeAreaInsets: proxy.safeAreaInsets,
                        division: proxy.activeFoldDivision,
                        verticalBarEdge: edge
                    )
                )
            }
        }
    }
}

/// Tells its content which edge the system uses for vertical bars, or `nil` where bars stay horizontal.
struct FoldableBarReader<Content: View>: View {
    @ViewBuilder var content: (HorizontalEdge?) -> Content

    var body: some View {
        #if os(iOS) && compiler(>=6.4)
        if #available(iOS 27.1, *) {
            SystemVerticalBarEdgeReader(content: content)
        } else {
            content(nil)
        }
        #else
        content(nil)
        #endif
    }
}

#if os(iOS) && compiler(>=6.4)
@available(iOS 27.1, *)
private struct SystemVerticalBarEdgeReader<Content: View>: View {
    @Environment(\.toolbarVerticalEdge) private var verticalBarEdge
    let content: (HorizontalEdge?) -> Content

    var body: some View {
        content(verticalBarEdge)
    }
}
#endif

// MARK: - Environment

private struct ChatBubbleMaxWidthKey: EnvironmentKey {
    static let defaultValue: CGFloat? = nil
}

extension EnvironmentValues {
    /// A cap on message bubble width that keeps every bubble on one side of an active fold.
    var chatBubbleMaxWidth: CGFloat? {
        get { self[ChatBubbleMaxWidthKey.self] }
        set { self[ChatBubbleMaxWidthKey.self] = newValue }
    }
}

// MARK: - View helpers

extension View {
    /// Keeps a bottom-anchored composer on the lower pane in laptop pose and on the trailing pane in book pose.
    @ViewBuilder
    func foldAwareComposerPane(_ arrangement: FoldAwareChatLayout.Arrangement) -> some View {
        switch arrangement {
        case .continuous:
            self
        case let .stacked(lowerRegionHeight, foldBand):
            padding(.top, foldBand)
                .frame(minHeight: lowerRegionHeight, alignment: .top)
        case let .sideBySide(paneWidth, _):
            frame(maxWidth: paneWidth)
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }

    /// Places a sheet beside the content it belongs to on wide displays, such as the inner display of
    /// iPhone Duo, where a centred sheet would sit on the hinge and hide the conversation it configures.
    @ViewBuilder
    func foldableTrailingSheetPlacement(_ isEnabled: Bool = true) -> some View {
        #if os(iOS) && compiler(>=6.4)
        if #available(iOS 27.0, *), isEnabled {
            presentationPlacement(.trailing)
        } else {
            self
        }
        #else
        self
        #endif
    }

    /// Shows a subtitle under the navigation title where the platform supports one.
    @ViewBuilder
    func foldableNavigationSubtitle(_ subtitle: String) -> some View {
        #if os(iOS)
        if #available(iOS 26.0, *) {
            navigationSubtitle(subtitle)
        } else {
            self
        }
        #else
        self
        #endif
    }
}

/// Lays out a full-screen media surface and its controls on opposite sides of an active fold.
struct FoldAwareMediaLayout<Media: View, Controls: View>: View {
    @ViewBuilder var media: () -> Media
    @ViewBuilder var controls: () -> Controls

    var body: some View {
        FoldableLayoutReader { layout in
            switch FoldAwareChatLayout.mediaArrangement(for: layout.division, in: layout.size) {
            case .continuous:
                VStack(spacing: 0) {
                    media()
                    controls()
                }
            case let .stacked(mediaHeight, foldBand):
                VStack(spacing: 0) {
                    media()
                        .frame(height: mediaHeight)
                    Color.clear
                        .frame(height: foldBand)
                    controls()
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                }
            case let .sideBySide(mediaWidth, foldBand):
                HStack(spacing: 0) {
                    media()
                        .frame(width: mediaWidth)
                    Color.clear
                        .frame(width: foldBand)
                    controls()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        }
    }
}

// MARK: - Toolbar helpers

enum FoldableToolbarPriority {
    /// Stays visible longest when a bar compresses, such as the vertical bar on iPhone Duo.
    case high
    /// Moves to the overflow menu first.
    case low
}

extension ToolbarContent {
    /// Orders items for the overflow menu where the system supports visibility priorities.
    @ToolbarContentBuilder
    func foldableVisibilityPriority(_ priority: FoldableToolbarPriority) -> some ToolbarContent {
        #if compiler(>=6.4)
        if #available(iOS 27.0, macOS 26.1, *) {
            if priority == .high {
                visibilityPriority(.high)
            } else {
                visibilityPriority(.low)
            }
        } else {
            self
        }
        #else
        self
        #endif
    }
}

/// Secondary actions that join the system overflow menu on iOS 27 and sit in a labelled menu before it.
///
/// Using the system menu keeps one ellipsis in the bar, which matters once iPhone Duo compresses
/// items into that same menu.
struct FoldableOverflowActions<Actions: View>: ToolbarContent {
    let title: String
    let systemImage: String
    var placement: ToolbarItemPlacement = .automatic
    @ViewBuilder var actions: () -> Actions

    var body: some ToolbarContent {
        #if os(iOS) && compiler(>=6.4)
        if #available(iOS 27.0, *) {
            ToolbarOverflowMenu {
                actions()
            }
        } else {
            ToolbarItem(placement: placement) {
                Menu(title, systemImage: systemImage) {
                    actions()
                }
            }
        }
        #else
        ToolbarItem(placement: placement) {
            Menu(title, systemImage: systemImage) {
                actions()
            }
        }
        #endif
    }
}
