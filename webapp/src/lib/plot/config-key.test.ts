import { describe, expect, it } from 'vitest';
import { planConfigSave } from './config-key';

const original = { site_id: 'i69', box_id: 'i69-mu1', source_id: 'i69-lj2', labjack_name: 'LJ2' };

describe('planConfigSave', () => {
    it('saves an edit that keeps the identity under the key it was loaded from', () => {
        const plan = planConfigSave({
            isAddingNew: false,
            editingKey: 'legacy-key.config',
            original,
            updated: { ...original, labjack_name: 'renamed' },
            existingKeys: ['legacy-key.config']
        });
        expect(plan).toEqual({ key: 'legacy-key.config', previousKey: null, conflict: false });
    });

    it('moves an edit whose site, box or source changed to the new key', () => {
        const plan = planConfigSave({
            isAddingNew: false,
            editingKey: 'i69.i69-mu1.i69-lj2.config',
            original,
            updated: { ...original, box_id: 'i69-mu2' },
            existingKeys: ['i69.i69-mu1.i69-lj2.config']
        });
        expect(plan).toEqual({
            key: 'i69.i69-mu2.i69-lj2.config',
            previousKey: 'i69.i69-mu1.i69-lj2.config',
            conflict: false
        });
    });

    it('flags a move or a new configuration onto a key that is already used', () => {
        const existingKeys = ['i69.i69-mu1.i69-lj2.config', 'i69.i69-mu2.i69-lj2.config'];
        expect(
            planConfigSave({
                isAddingNew: false,
                editingKey: 'i69.i69-mu1.i69-lj2.config',
                original,
                updated: { ...original, box_id: 'i69-mu2' },
                existingKeys
            }).conflict
        ).toBe(true);
        expect(
            planConfigSave({ isAddingNew: true, editingKey: '', original: null, updated: original, existingKeys }).conflict
        ).toBe(true);
    });

    it('builds the key of a new configuration from its identity', () => {
        const plan = planConfigSave({ isAddingNew: true, editingKey: '', original: null, updated: original, existingKeys: [] });
        expect(plan).toEqual({ key: 'i69.i69-mu1.i69-lj2.config', previousKey: null, conflict: false });
    });
});

describe('planConfigSave with an empty source', () => {
    it('keys a new configuration without source or name by its asset number', () => {
        const plan = planConfigSave({
            isAddingNew: true,
            editingKey: '',
            original: null,
            updated: { site_id: 'i69', box_id: 'i69-mu1', source_id: '', labjack_name: '', asset_number: 7 },
            existingKeys: []
        });
        expect(plan.key).toBe('i69.i69-mu1.asset007.config');
    });

    it('does not move an existing configuration saved under the old unknown-source key', () => {
        const identity = { site_id: 'i69', box_id: 'i69-mu1', source_id: '', labjack_name: '', asset_number: 7 };
        const plan = planConfigSave({
            isAddingNew: false,
            editingKey: 'i69.i69-mu1.unknown-source.config',
            original: identity,
            updated: { ...identity },
            existingKeys: ['i69.i69-mu1.unknown-source.config']
        });
        expect(plan).toEqual({ key: 'i69.i69-mu1.unknown-source.config', previousKey: null, conflict: false });
    });
});
