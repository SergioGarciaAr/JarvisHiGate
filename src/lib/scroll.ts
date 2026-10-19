// Follow-the-latest logic for the conversation log, kept pure for testing.

/** Distance from the bottom (px) still considered "at the end". */
export const NEAR_BOTTOM_PX = 64;

export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

export function isNearBottom(m: ScrollMetrics, threshold = NEAR_BOTTOM_PX): boolean {
  return m.scrollHeight - m.scrollTop - m.clientHeight <= threshold;
}

export interface FollowState {
  /** Keep the view pinned to the latest message. */
  follow: boolean;
  /** New content arrived while the user was reading earlier messages. */
  unseen: boolean;
}

export const initialFollow: FollowState = { follow: true, unseen: false };

/**
 * Scroll events are dispatched asynchronously. A programmatic pin can be
 * reported after more content has arrived, which would look like the user
 * moved away from the end. Events at the position we set are ignored.
 */
export function isOwnScroll(scrollTop: number, pinnedTop: number | null): boolean {
  return pinnedTop !== null && Math.abs(scrollTop - pinnedTop) <= 1;
}

/** The user (or the browser) scrolled: follow only when near the end. */
export function afterScroll(state: FollowState, m: ScrollMetrics): FollowState {
  const near = isNearBottom(m);
  return { follow: near, unseen: near ? false : state.unseen };
}

/** Content or viewport size changed. Returns whether to pin to the bottom. */
export function afterResize(state: FollowState, contentGrew: boolean): { state: FollowState; pin: boolean } {
  if (state.follow) return { state: { follow: true, unseen: false }, pin: true };
  return { state: { follow: false, unseen: state.unseen || contentGrew }, pin: false };
}

/** The user sent a message or asked to jump to the latest one. */
export function jumpToLatest(): { state: FollowState; pin: boolean } {
  return { state: { follow: true, unseen: false }, pin: true };
}
