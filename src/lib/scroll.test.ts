import { describe, expect, it } from "vitest";
import { afterResize, afterScroll, initialFollow, isNearBottom, isOwnScroll, jumpToLatest, NEAR_BOTTOM_PX } from "./scroll";

const at = (scrollTop: number, scrollHeight = 3000, clientHeight = 500) => ({ scrollTop, scrollHeight, clientHeight });

describe("follow the latest message", () => {
  it("short conversation (no overflow) counts as at the end", () => {
    expect(isNearBottom(at(0, 400, 500))).toBe(true);
  });

  it("near-bottom threshold", () => {
    expect(isNearBottom(at(2500))).toBe(true);
    expect(isNearBottom(at(2500 - NEAR_BOTTOM_PX))).toBe(true);
    expect(isNearBottom(at(2500 - NEAR_BOTTOM_PX - 1))).toBe(false);
  });

  it("new content while at the end keeps following (streaming)", () => {
    const r = afterResize(initialFollow, true);
    expect(r.pin).toBe(true);
    expect(r.state).toEqual({ follow: true, unseen: false });
  });

  it("new content while reading older messages does not move the view and flags unseen", () => {
    const reading = afterScroll(initialFollow, at(800));
    expect(reading.follow).toBe(false);
    const r = afterResize(reading, true);
    expect(r.pin).toBe(false);
    expect(r.state.unseen).toBe(true);
  });

  it("window resize while reading neither pins nor flags unseen", () => {
    const reading = afterScroll(initialFollow, at(800));
    expect(afterResize(reading, false)).toEqual({ state: { follow: false, unseen: false }, pin: false });
  });

  it("scrolling back to the end resumes following and clears the indicator", () => {
    const reading = afterResize(afterScroll(initialFollow, at(800)), true).state;
    expect(afterScroll(reading, at(2490))).toEqual({ follow: true, unseen: false });
  });

  it("jumping to latest (indicator or sending a message) pins", () => {
    expect(jumpToLatest()).toEqual({ state: { follow: true, unseen: false }, pin: true });
  });

  it("ignores late scroll events caused by our own pin", () => {
    // Pinned at 2500, content then grew by 84px before the event was dispatched.
    expect(isOwnScroll(2500, 2500)).toBe(true);
    expect(isOwnScroll(2500.5, 2500)).toBe(true);
    expect(isOwnScroll(2300, 2500)).toBe(false);
    expect(isOwnScroll(2500, null)).toBe(false);
  });
});
