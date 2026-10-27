# iPhone Duo

See the [Apple platforms README](../../apps/ios/README.md) for building, testing and releasing the apps.

Babel Bridge follows Apple's [Designing for iPhone Duo](https://developer.apple.com/design/human-interface-guidelines/designing-for-iphone-duo)
guidance and the [Preparing your app for iPhone Duo](https://developer.apple.com/documentation/technologyoverviews/preparing-your-app-for-iphone-duo)
overview. The same functionality is available on both displays and in every pose;
only the arrangement changes.

Outer display (device closed, compact width):

- Toolbars and the navigation controls move to the side of the display. Every
  toolbar item has a title and a symbol, so the system can present it vertically.
  Messages, Chats, Settings, Updates, Gallery and the topic filter of the
  Messages feed carry a high visibility priority and stay in the bar when it
  compresses. In a conversation, Gallery and the ellipsis menu of secondary
  actions (search, starred filter, topic filter, cost, settings) form their
  own group at the right of the header, split from Messages, Chats and Settings
  by a spacer. In a vertical bar they are leading items, so, like the Messages
  feed's topic filter, they get their own glass button above the capsule of
  destinations instead of joining it.
- Chat headers fade into the wallpaper instead of drawing a bar: iOS 26 blurs
  scrolled content softly under a clear navigation bar, the Messages feed keeps
  its topic filter in the bar rather than under the title, and the conversation
  header in the content sits on a frosted fade that is strongest at the top of
  the display, eases through the header so a scrolled bubble shows only faintly
  behind the name, and dissolves over the row beneath it, as WhatsApp's header
  does. The composer floats in the bottom safe area on both screens, so
  messages scroll behind its glass controls. An active topic shows as a
  clearable pill above the timeline, like the starred filter.
- In the unified Messages feed each chat's heading (avatar, name, date) is a
  pinned section header, so the person being read stays at the top of the feed
  while their messages scroll. The grouping rule is
  `UnifiedFeedSectionBuilder` in `APIModels.swift`, covered by
  `UnifiedFeedSectionTests`.
- The Updates bell keeps its drawn unread count in horizontal bars and switches
  to a system badge in a vertical bar, where only title-and-symbol items appear.
- The conversation header (avatar, name, translation status) lives in the
  content whenever bars are vertical, because a custom principal item never
  joins a vertical bar. Chats keeps its single-pane split view.

Inner display (device open, regular width):

- Chats shows the list and the conversation side by side, like Mail, and the
  system adapts the column widths to the hinge.
- Conversation settings, cost and topic sheets opened from a conversation use a
  trailing presentation placement on iOS 27, so they cover the conversation they
  configure rather than the centre of the display.
- Gallery uses an even number of columns when the device is folded like a book
  and widens the middle spacing into a gutter over the hinge, so no tile is
  split.

Partially open poses (the hinge is an active reserved division):

- Laptop pose (horizontal fold): the timeline reads on the upper pane and ends
  at the fold; the composer, reply preview and photo-send progress rest on the
  lower pane directly under the fold band. Chats and the unified Messages feed
  behave the same way. When the keyboard leaves no usable lower pane the
  ordinary layout returns.
- Book pose (vertical fold): every message bubble stays within one pane, so no
  message runs across the hinge, and the composer sits on the trailing pane.
- Full-screen photos and gallery media fill one pane while their captions and
  controls fill the other, following the fold's direction.

Flat displays, closed devices and every other iPhone, iPad and Mac keep the
layout they have today. Rotation, Split View multitasking and the camera
occlusions are handled by the system containers the app already uses.

Implementation notes:

- `WhatsAppTranslator/Design/FoldableLayoutModel.swift` holds the pure layout
  rules (`FoldDivision`, `FoldAwareChatLayout`). It has no SwiftUI dependency so
  `SharedTests/FoldableLayoutTests.swift` runs in both unit-test targets.
- `WhatsAppTranslator/Design/FoldableDisplaySupport.swift` wraps the iOS 27 and
  27.1 APIs: reserved regions through `GeometryProxy`, `toolbarVerticalEdge`,
  `visibilityPriority` and `presentationPlacement`. Each
  use is behind `#if compiler(>=6.4)` (Swift 6.4 ships with Xcode 27) plus a
  runtime availability check, so Xcode 26.6 compiles the same sources without
  any iPhone Duo behaviour and the deployment target stays iOS 18.
- Build with Xcode 27.1 or later to get the iPhone Duo behaviour. Apple notes
  that apps built with Xcode 26 and earlier do not extend under the status bar
  and camera on iPhone Duo, so point the Xcode Cloud workflow at a released
  Xcode 27.1 environment before shipping. The GitHub Actions check stays on
  Xcode 26.6 until the `macos-26` runner image provides Xcode 27.1; the guarded
  code paths are therefore only compiled by Xcode 27.1 builds.

Validation checklist for a Duo simulator (Xcode 27.1 Device Hub poses) or device:

- Closed, portrait and landscape: vertical bar shows Messages, Chats, Settings,
  Updates (badge), Gallery, the conversation ellipsis menu and the Messages
  feed's topic filter; the ellipsis menu lists the conversation actions, including Filter by topic;
  the conversation header is in the content.
- Open flat, portrait and landscape: two-column Chats; trailing conversation
  settings and cost sheets; Gallery grid with four or six columns.
- Book pose: bubbles and the composer stay off the hinge in Chats and
  Messages; Gallery shows a gutter over the hinge; a photo shows on the left
  pane with its actions on the right.
- Laptop pose: the timeline ends at the fold and the composer sits just below
  it; opening the keyboard restores the ordinary layout; media viewers put the
  photo or video above the fold and the controls below.
- Transitions: open and close during a conversation with a draft, a focused
  reply and a photo send in progress; nothing is lost and no control disappears.

This change was prepared without an iPhone Duo simulator or device, so the
checklist above still needs to be run and recorded.
