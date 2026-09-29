import { describe, expect, it } from 'vitest';
import { parseAssetNumberParam } from './route';

describe('parseAssetNumberParam', () => {
    it('accepts non-negative integers', () => {
        expect(parseAssetNumberParam('1456')).toBe(1456);
        expect(parseAssetNumberParam('0')).toBe(0);
    });

    it('rejects anything else', () => {
        for (const raw of ['', 'abc', '12abc', '-3', '1.5', ' ', undefined, null, '99999999999999999999']) {
            expect(parseAssetNumberParam(raw)).toBeNull();
        }
    });
});
