import { describe, expect, it } from 'vitest';
import {
    channelStatusLabel,
    defaultAxisSettings,
    defaultTriggerSettings,
    deriveAutoTimeWindowSec,
    isTriggerMode,
    pickInitialPlotChannels,
    plotConfigFor,
    requiredBufferPoints,
    snapshotKeepMs,
    snapshotRateHz,
    streamProblemText,
    togglePlotSelection
} from './channel';
import type { TriggerCapture } from './trigger';

describe('deriveAutoTimeWindowSec', () => {
    it('shows about 1000 samples, at least 0.05 s', () => {
        expect(deriveAutoTimeWindowSec(1000)).toBe(1);
        expect(deriveAutoTimeWindowSec(2000)).toBe(0.5);
        expect(deriveAutoTimeWindowSec(50000)).toBe(0.05);
    });
    it('falls back to 1 s for a missing or invalid rate', () => {
        expect(deriveAutoTimeWindowSec(0)).toBe(1);
        expect(deriveAutoTimeWindowSec(-5)).toBe(1);
        expect(deriveAutoTimeWindowSec(Number.NaN)).toBe(1);
    });
});

describe('default settings', () => {
    it('starts in auto Y with -1..1 limits and a rising trigger at 0', () => {
        expect(defaultAxisSettings(0.5)).toEqual({ autoY: true, yMin: -1, yMax: 1, xWindowSec: 0.5, invertX: false, invertY: false });
        expect(defaultTriggerSettings(0.5)).toEqual({ type: 'rising', threshold: 0, preTriggerPercent: 40, postTriggerWindowSec: 0.5 });
    });
});

describe('plot selection', () => {
    it('picks the first two enabled channels', () => {
        expect([...pickInitialPlotChannels([3, 1, 2])]).toEqual([3, 1]);
        expect([...pickInitialPlotChannels([])]).toEqual([]);
    });
    it('keeps a previous selection that is still enabled, in config order', () => {
        expect([...pickInitialPlotChannels([0, 1, 2, 3], new Set([3, 2]))]).toEqual([2, 3]);
        expect([...pickInitialPlotChannels([0, 1, 2], new Set([2, 9]))]).toEqual([2, 0]);
    });
    it('refuses a third channel and removes unticked ones', () => {
        expect(togglePlotSelection(new Set([1, 2]), 3, true)).toBeNull();
        expect([...togglePlotSelection(new Set([1]), 3, true)!]).toEqual([1, 3]);
        expect([...togglePlotSelection(new Set([1, 2]), 1, false)!]).toEqual([2]);
    });
});

describe('requiredBufferPoints', () => {
    const axis = defaultAxisSettings(1);
    const trigger = defaultTriggerSettings(1);
    it('covers twice the auto window with 10 % and one read of headroom', () => {
        expect(requiredBufferPoints(1000, 100, 1, [])).toBe(Math.ceil(1000 * 2 * 1.1) + 100);
        expect(requiredBufferPoints(1000, 0, 1, [])).toBe(Math.ceil(1000 * 2 * 1.1) + 1);
    });
    it('grows with a wider X window or a longer trigger window', () => {
        expect(requiredBufferPoints(1000, 1, 1, [{ axis: { ...axis, xWindowSec: 5 }, trigger }])).toBe(Math.ceil(1000 * 10 * 1.1) + 1);
        // 90 % pre-trigger of a 3 s post window: pre = 27 s, total 30 s.
        const long = { ...trigger, preTriggerPercent: 90, postTriggerWindowSec: 3 };
        const points = requiredBufferPoints(1000, 1, 1, [{ axis, trigger: long }]);
        expect(points).toBeGreaterThanOrEqual(33001);
        expect(points).toBeLessThanOrEqual(33002);
    });
    it('uses at least 0.1 s per channel X window', () => {
        expect(requiredBufferPoints(1000, 1, 0.01, [{ axis: { ...axis, xWindowSec: 0.01 }, trigger: { ...trigger, postTriggerWindowSec: 0.01, preTriggerPercent: 0 } }]))
            .toBe(Math.ceil(1000 * 0.2 * 1.1) + 1);
    });
});

describe('snapshotKeepMs', () => {
    it('keeps 1.25 X windows, at least 0.25 s', () => {
        expect(snapshotKeepMs(2, 'free_run', undefined)).toBe(2500);
        expect(snapshotKeepMs(0.1, 'free_run', undefined)).toBe(250);
    });
    it('keeps the pre-trigger window plus 0.25 s in trigger modes', () => {
        const trigger = { type: 'rising' as const, threshold: 0, preTriggerPercent: 50, postTriggerWindowSec: 4 };
        expect(snapshotKeepMs(1, 'trigger_normal', trigger)).toBe(4250);
        expect(snapshotKeepMs(1, 'free_run', trigger)).toBe(1250);
    });
});

describe('plotConfigFor', () => {
    const trigger = defaultTriggerSettings(1);
    const live = [{ timestamp: 1, value: 1 }];
    const capture: TriggerCapture = {
        data: [{ timestamp: 5, value: 2 }],
        triggerTime: 5,
        startTime: 4,
        endTime: 6,
        preWindowSec: 0.2,
        postWindowSec: 0.3,
        complete: false
    };
    it('freezes on a capture in a trigger mode', () => {
        const config = plotConfigFor('trigger_single', live, capture, trigger);
        expect(config).toEqual({
            mode: 'frozen', data: live, frozenData: capture.data, isTriggered: true, triggerTime: 5,
            frozenPreWindowSec: 0.2, frozenPostWindowSec: 0.3, frozenCollecting: true
        });
    });
    it('runs continuously otherwise, with the trigger windows of the settings', () => {
        const config = plotConfigFor('free_run', live, capture, trigger);
        expect(config.mode).toBe('continuous');
        expect(config.frozenData).toBeUndefined();
        expect(config.isTriggered).toBe(false);
        expect(config.frozenPostWindowSec).toBe(1);
        expect(config.frozenPreWindowSec).toBeCloseTo(1 * 0.4 / 0.6);
        expect(plotConfigFor('trigger_normal', live, null, trigger).mode).toBe('continuous');
    });
});

describe('labels', () => {
    it('describes the channel state', () => {
        expect(isTriggerMode('free_run')).toBe(false);
        expect(isTriggerMode('trigger_normal')).toBe(true);
        expect(channelStatusLabel('free_run', false, true)).toBe('Running');
        expect(channelStatusLabel('trigger_normal', false, true)).toBe('Pre-buffering');
        expect(channelStatusLabel('trigger_single', true, true)).toBe('Triggered');
        expect(channelStatusLabel('trigger_single', true, false)).toBe('Armed');
    });
    it('computes the snapshot rate', () => {
        expect(snapshotRateHz([])).toBe(0);
        expect(snapshotRateHz([{ timestamp: 0, value: 0 }])).toBe(0);
        expect(snapshotRateHz([{ timestamp: 0, value: 0 }, { timestamp: 1, value: 0 }, { timestamp: 2, value: 0 }])).toBe(1000);
    });
    it('summarises stream problems', () => {
        const none = { dropped: 3, gaps: 0, resets: 0, skipped: 0, decodeErrors: 0 };
        expect(streamProblemText(1, none)).toBeNull();
        expect(streamProblemText(1, undefined)).toBeNull();
        expect(streamProblemText(1, { ...none, gaps: 2 })).toBe('Ch1: 2 gaps');
        expect(streamProblemText(4, { dropped: 0, gaps: 0, resets: 1, skipped: 5, decodeErrors: 2 }))
            .toBe('Ch4: 0 gaps, 1 resets, 5 dup samples, 2 bad msgs');
    });
});
