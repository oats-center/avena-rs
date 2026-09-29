/**
 * Chooses the KV key a LabJack configuration is saved under.
 *
 * A configuration lives at `<site>.<box>.<source>.config` (see `labjackConfigKey`).
 * When an edit changes the site, box or source, the configuration must move to the key
 * of its new identity; otherwise the stored key and the content disagree.
 *
 * @module
 */
import { labjackConfigKey } from '../subjects';

/** Identity fields that make up a configuration's key. */
export interface ConfigIdentity {
    site_id?: string | null;
    box_id?: string | null;
    source_id?: string | null;
    labjack_name?: string | null;
    /** Source fallback `asset<NNN>` when `source_id` and `labjack_name` are empty. */
    asset_number?: number | null;
}

/** Result of {@link planConfigSave}. */
export interface ConfigSavePlan {
    /** Key to write the configuration to. */
    key: string;
    /** Key the configuration moves away from, to delete after the write succeeds. */
    previousKey: string | null;
    /** `key` already holds a different configuration; saving would overwrite it. */
    conflict: boolean;
}

/**
 * Decides where to save a configuration from the edit form.
 *
 * - New configuration: the key of its identity.
 * - Edit that keeps the identity (same derived key as before the edit): the key it was
 *   loaded from, even if that key does not follow the current naming.
 * - Edit that changes site, box or source: the key of the new identity, with the old
 *   key returned as `previousKey`.
 *
 * `conflict` is set when the target key is taken by another configuration.
 *
 * @param options.isAddingNew - The form was opened with "Add New LabJack".
 * @param options.editingKey - Key the edited configuration was loaded from.
 * @param options.original - The configuration as loaded, before the edit.
 * @param options.updated - The configuration to save.
 * @param options.existingKeys - Keys of all loaded configurations.
 */
export function planConfigSave(options: {
    isAddingNew: boolean;
    editingKey: string;
    original: ConfigIdentity | null;
    updated: ConfigIdentity;
    existingKeys: Iterable<string>;
}): ConfigSavePlan {
    const existing = new Set(options.existingKeys);
    const newIdentityKey = labjackConfigKey(options.updated);

    if (options.isAddingNew || !options.editingKey) {
        return { key: newIdentityKey, previousKey: null, conflict: existing.has(newIdentityKey) };
    }

    const identityChanged =
        !options.original || labjackConfigKey(options.original) !== newIdentityKey;
    if (!identityChanged || newIdentityKey === options.editingKey) {
        return { key: options.editingKey, previousKey: null, conflict: false };
    }

    return {
        key: newIdentityKey,
        previousKey: options.editingKey,
        conflict: existing.has(newIdentityKey)
    };
}
