import { error } from '@sveltejs/kit';
import { parseAssetNumberParam } from '$lib/plot/route';
import type { PageLoad } from './$types';

/**
 * Rejects an `asset_number` that is not a non-negative integer with a 404, so SvelteKit
 * shows the shared error page instead of the plot page. A valid number is passed on;
 * the page itself loads the config and reports a LabJack that is not found.
 */
export const load: PageLoad = ({ params }) => {
    if (parseAssetNumberParam(params.asset_number) === null) {
        error(404, `"${params.asset_number}" is not a valid asset number. Open a LabJack's plots from the LabJacks page.`);
    }
    return {};
};
