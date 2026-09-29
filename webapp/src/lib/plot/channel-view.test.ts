import { describe, expect, it } from 'vitest';
import { ChannelView } from './channel-view.svelte';
import type { TriggerCapture } from './trigger';

const capture = (triggerTime: number): TriggerCapture => ({
    data: [{ timestamp: triggerTime, value: 1 }],
    triggerTime,
    startTime: triggerTime - 100,
    endTime: triggerTime + 100,
    preWindowSec: 0.1,
    postWindowSec: 0.1,
    complete: false
});

describe('ChannelView', () => {
    it('starts in free run with the default settings', () => {
        const view = new ChannelView(3, 0.5);
        expect(view.channel).toBe(3);
        expect(view.mode).toBe('free_run');
        expect(view.axis).toEqual({ autoY: true, yMin: -1, yMax: 1, xWindowSec: 0.5, invertX: false, invertY: false });
        expect(view.trigger.postTriggerWindowSec).toBe(0.5);
        expect(view.liveData).toEqual([]);
        expect(view.live.buffer).toEqual([]);
    });

    it('keeps the capture between the trigger modes and clears it otherwise', () => {
        const view = new ChannelView(0, 1);
        expect(view.setMode('trigger_normal')).toBe(true);
        expect(view.prebufferReady).toBe(false);
        view.setCapture(capture(1000));
        expect(view.triggered).toBe(true);
        expect(view.triggerTime).toBe(1000);
        view.setMode('trigger_single');
        expect(view.capture?.triggerTime).toBe(1000);
        expect(view.setMode('trigger_single')).toBe(false);
        view.setMode('free_run');
        expect(view.capture).toBeNull();
        expect(view.triggered).toBe(false);
        expect(view.triggerTime).toBe(0);
        expect(view.prebufferReady).toBe(true);
    });

    it('computes the pre-buffer flag from the live buffer when entering a trigger mode', () => {
        const view = new ChannelView(0, 1);
        // 40 % pre-trigger of a 1 s post window: about 0.667 s needed.
        view.live.buffer.push({ timestamp: 0, value: 0 }, { timestamp: 700, value: 0 });
        view.setMode('trigger_normal');
        expect(view.prebufferReady).toBe(true);
    });

    it('rejects bad axis limits with a message and keeps the old value', () => {
        const view = new ChannelView(0, 1);
        expect(view.commitAxisLimit('yMin', '5')).toBe('-1');
        expect(view.axisError).toBe('Y Min must be less than Y Max.');
        expect(view.axis.yMin).toBe(-1);
        expect(view.commitAxisLimit('yMin', '0')).toBeNull();
        expect(view.axisError).toBe('');
        expect(view.axis.yMin).toBe(0);
        expect(view.commitAxisLimit('xWindowSec', '0')).toBe('1');
        view.updateAxis({ invertX: true });
        expect(view.axisError).toBe('');
        expect(view.axis.invertX).toBe(true);
    });

    it('checks trigger inputs', () => {
        const view = new ChannelView(0, 1);
        expect(view.commitTriggerNumber('threshold', '0')).toBeNull();
        expect(view.commitTriggerNumber('threshold', '-2.5')).toBeNull();
        expect(view.trigger.threshold).toBe(-2.5);
        expect(view.commitTriggerNumber('threshold', 'x')).toBe('-2.5');
        expect(view.commitTriggerNumber('postTriggerWindowSec', '0.001')).toBe('1');
        view.setPreTriggerPercent('120');
        expect(view.trigger.preTriggerPercent).toBe(95);
        view.setPreTriggerPercent('abc');
        expect(view.trigger.preTriggerPercent).toBe(0);
        view.setTriggerEdge('falling');
        expect(view.trigger.type).toBe('falling');
    });

    it('starts a fresh trace on reset', () => {
        const view = new ChannelView(0, 1);
        const old = view.live;
        view.setCapture(capture(5));
        view.liveData = [{ timestamp: 1, value: 1 }];
        view.prebufferReady = true;
        view.resetStream();
        expect(view.live).not.toBe(old);
        expect(view.capture).toBeNull();
        expect(view.triggered).toBe(false);
        expect(view.prebufferReady).toBe(false);
        expect(view.liveData).toEqual([]);
    });
});
