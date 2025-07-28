<script lang='ts'>
    import { onMount } from "svelte";

  let {top, left, type_modal}: {top: number, left: number, type_modal: HTMLDialogElement } = $props();

</script>

<ul class="menu custom_menu bg-primary rounded-box w-56" style="position: fixed; top: {top - 60}px; left: {left}px; z-index: 999; height: 60px">
  <li><button onclick={() => type_modal?.showModal()} class="text-black">Edit Sensor Type</button></li>
</ul>