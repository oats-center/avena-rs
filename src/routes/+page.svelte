<script>
    import { onMount } from "svelte";

  let natsMessages = $state([]);
  let sentMessage = $state("");

  async function getNatsMessages() {
    const response = await fetch('/nats');
    if (response.ok) {
      const result = await response.json();
      natsMessages = [result.message]
    } else {
      console.error('Failed to fetch messages:', response.statusText);
    }
  }
  async function sendNatsMessage() {
    console.log(sentMessage);
    const response = await fetch("/nats", {
      headers: {"Content-Type": "application/json"},
      method: "POST",
      body: JSON.stringify({message: sentMessage})
    })
    if(response.ok){
      console.log("Message Sent Successfully");
    } else {
      console.error("Failed to send message: ", response.statusText);
    }
    sentMessage = "";  
  }
</script>

<div class="flex space">
  <div class="mx-10">
    <button class="btn btn-primary" onclick={getNatsMessages}>NATS Get Message</button>
    <ul>
      {#each natsMessages as message}
        <li>{message}</li>
      {/each}
    </ul>
  </div>
  <div >
    <input type="text" placeholder="Send A Message" bind:value={sentMessage} class="input input-bordered w-full max-w-xs"/>
    <button class="btn btn-primary" onclick={sendNatsMessage} >Send Message</button>
  </div>
  </div>


