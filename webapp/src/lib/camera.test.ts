import { describe, expect, it } from 'vitest';
import { liveSeekTarget, MAX_LAG_SECONDS } from './camera';

describe('liveSeekTarget', () => {
    it('keeps playing while inside the buffer and close to its end', () => {
        expect(liveSeekTarget(10, 0, 10 + MAX_LAG_SECONDS)).toBeNull();
        expect(liveSeekTarget(10, 9, 10.2)).toBeNull();
    });

    it('jumps to just before the newest frame when too far behind', () => {
        expect(liveSeekTarget(10, 0, 14)).toBeCloseTo(13.8);
    });

    it('jumps into the buffer when playback is before it', () => {
        expect(liveSeekTarget(0, 2.5, 3)).toBeCloseTo(2.8);
        expect(liveSeekTarget(0, 2.9, 3)).toBeCloseTo(2.9);
    });
});
