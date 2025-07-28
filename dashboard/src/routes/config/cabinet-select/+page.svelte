<script lang="ts">
    import { onMount } from "svelte";
    import {
        NatsService,
        connect,
        getKeys,
        getKeyValue,
    } from "$lib/nats.svelte";
    import { goto } from "$app/navigation";

    type Cabinet = {
        id: string;
        status: string;
    };

    let serverName: string | null;
    let nats: NatsService | null;
    let cabinetKeys = $state<string[]>([]);
    let cabinets = $state<Cabinet[]>([]);
    let loading = $state<boolean>(true);

    //initializes the nats connection and gets all cabinet vals
    async function initialize() {
        if (serverName) nats = await connect(serverName);
        if (nats) {
            let keys = await getKeys(nats, "all_cabinets");
            for (const key of keys) {
                let values = await getCabinet("all_cabinets", key);
                cabinets.push(values);
                console.log(values);
            }
            loading = false;
        } else {
            console.log("No Nats Connection");
        }
    }

    //gets the value of one cabinet
    async function getCabinet(bucket: string, key: string): Promise<Cabinet> {
        if (!nats) throw new Error("Nats connection is not initialized");
        let cabinet = {
            id: key,
            status: "",
        };
        let status = await getKeyValue(nats, bucket, key);
        cabinet.status = JSON.parse(status).status;
        return cabinet;
    }

    //once selected, sets the selectedCabinet to session storage and redirects to labjack config page
    function selectConfig(selectedCabinet: string) {
        sessionStorage.setItem("selectedCabinet", selectedCabinet);
        goto("/config/lj-config");
    }

    //gets the server name from session storage & initalizes
    onMount(() => {
        serverName = sessionStorage.getItem("serverName");
        initialize();
    });
</script>

{#if loading}
    <div class="loading-overlay">
        <span class="loading loading-spinner loading-lg"></span>
    </div>
{:else if cabinets !== null && cabinetKeys !== null}
    <div class="flex flex-col items-center w-full px-10">
        <h1>Select Roadside Cabinet</h1>
        <div
            class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-5 w-full max-w-[80%]"
        >
            {#each cabinets as cabinet}
                <div class="card bg-primary shadow-xl text-neutral p-4">
                    <div class="card-body space-y-4">
                        <h2 class="card-title text-center">{cabinet["id"]}</h2>
                        <p class="pl-2 mt-2">
                            <strong>Status:</strong>
                            {cabinet.status}
                        </p>
                        <div class="mt-3 flex justify-center">
                            <button
                                class="btn btn-outline btn-success"
                                onclick={() => selectConfig(cabinet.id)}
                            >
                                Select Cabinet
                            </button>
                        </div>
                    </div>
                </div>
            {/each}
        </div>
    </div>
{/if}
