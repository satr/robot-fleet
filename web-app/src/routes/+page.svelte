<script lang="ts">
  import { onMount } from 'svelte';
  import {
    backendWsUrl,
    deleteRobot,
    fetchRobots,
    sendRobotCommand
  } from '$lib/api';
  import type { Robot, RobotStreamMessage } from '$lib/types';

  let robots: Robot[] = [];
  let loading = true;
  let error = '';
  let pendingAction = '';

  const commandButtons = [
    { icon: '→', label: '+X', command: 'move', payload: { axis: 'x', delta: 1 } },
    { icon: '←', label: '-X', command: 'move', payload: { axis: 'x', delta: -1 } },
    { icon: '↑', label: '+Y', command: 'move', payload: { axis: 'y', delta: 1 } },
    { icon: '↓', label: '-Y', command: 'move', payload: { axis: 'y', delta: -1 } },
    { icon: '▶', label: 'Run', command: 'run', payload: {} },
    { icon: '■', label: 'Stop', command: 'stop', payload: {} }
  ];

  onMount(() => {
    void loadRobots();
    const socket = new WebSocket(`${backendWsUrl()}/robots/stream`);
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data) as RobotStreamMessage;
      if (message.event_type === 'robot_deleted' && message.robot_id) {
        robots = robots.filter((robot) => robot.robot_id !== message.robot_id);
      }
      if (message.event_type === 'robot_updated' && message.robot) {
        upsertRobot(message.robot);
      }
    };
    socket.onerror = () => {
      error = 'Live robot updates are disconnected.';
    };
    return () => socket.close();
  });

  async function loadRobots() {
    loading = true;
    error = '';
    try {
      robots = await fetchRobots();
    } catch (err) {
      error = err instanceof Error ? err.message : 'Failed to load robots.';
    } finally {
      loading = false;
    }
  }

  function upsertRobot(robot: Robot) {
    const index = robots.findIndex((item) => item.robot_id === robot.robot_id);
    if (index === -1) {
      robots = [...robots, robot].sort((left, right) =>
        left.robot_id.localeCompare(right.robot_id)
      );
      return;
    }
    robots = robots.map((item) => (item.robot_id === robot.robot_id ? robot : item));
  }

  async function runCommand(robot: Robot, command: string, payload = {}) {
    pendingAction = `${robot.robot_id}:${command}`;
    error = '';
    try {
      await sendRobotCommand(robot.robot_id, command, payload);
    } catch (err) {
      error = err instanceof Error ? err.message : `Failed to send ${command}.`;
    } finally {
      pendingAction = '';
    }
  }

  async function removeRobot(robot: Robot) {
    if (robot.status !== 'offline') {
      return;
    }
    pendingAction = `${robot.robot_id}:delete`;
    error = '';
    try {
      await deleteRobot(robot.robot_id);
      robots = robots.filter((item) => item.robot_id !== robot.robot_id);
    } catch (err) {
      error = err instanceof Error ? err.message : 'Failed to delete robot.';
    } finally {
      pendingAction = '';
    }
  }

  function coordinate(value: number | null) {
    return value === null ? 'n/a' : value.toFixed(2);
  }

  function speed(value: number | null) {
    return value === null ? 'n/a' : `${value.toFixed(2)} cm/s`;
  }

  function heading(value: number | null) {
    return value === null ? 'n/a' : `${value.toFixed(1)}°`;
  }
</script>

<svelte:head>
  <title>Robot Fleet</title>
</svelte:head>

<main class="shell">
  <section class="hero">
    <div>
      <p class="eyebrow">Robot Fleet</p>
      <h1>Fleet control</h1>
      <p class="summary">Live robot state from the Rust backend with REST commands and WebSocket updates.</p>
    </div>
    <button class="secondary" on:click={loadRobots}>Refresh</button>
  </section>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  {#if loading}
    <p class="empty">Loading robots...</p>
  {:else if robots.length === 0}
    <p class="empty">No robots have reported state yet.</p>
  {:else}
    <section class="grid" aria-label="Robots">
      {#each robots as robot (robot.robot_id)}
        <article class="card">
          <header>
            <div>
              <h2>{robot.name}</h2>
              <p>{robot.robot_id}</p>
            </div>
            <span class:online={robot.status === 'online'} class:stale={robot.status === 'stale'} class:offline={robot.status === 'offline'}>
              {robot.status}
            </span>
          </header>

          <dl>
            <div>
              <dt>Battery</dt>
              <dd>{robot.battery_level.toFixed(1)}%</dd>
            </div>
            <div>
              <dt>Position</dt>
              <dd>x {coordinate(robot.position_x)} cm, y {coordinate(robot.position_y)} cm</dd>
            </div>
            <div>
              <dt>Velocity</dt>
              <dd>{speed(robot.velocity_cm_s)}</dd>
            </div>
            <div>
              <dt>Direction</dt>
              <dd>{heading(robot.direction_degrees)}</dd>
            </div>
            <div>
              <dt>Current command</dt>
              <dd>{robot.current_command ?? robot.current_mission ?? 'none'}</dd>
            </div>
            <div>
              <dt>Command status</dt>
              <dd>{robot.current_command_status ?? 'idle'}</dd>
            </div>
          </dl>

          <div class="controls" aria-label={`Controls for ${robot.name}`}>
            {#each commandButtons as button}
              <button
                title={button.label}
                disabled={pendingAction !== ''}
                on:click={() => runCommand(robot, button.command, button.payload)}
              >
                <span>{button.icon}</span>
                {button.label}
              </button>
            {/each}
            <button
              class="danger"
              title="Delete offline robot"
              disabled={robot.status !== 'offline' || pendingAction !== ''}
              on:click={() => removeRobot(robot)}
            >
              <span>🗑</span>
              Delete
            </button>
          </div>
        </article>
      {/each}
    </section>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    background: #0f172a;
    color: #e2e8f0;
    font-family:
      Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  }

  .shell {
    max-width: 1180px;
    margin: 0 auto;
    padding: 32px 20px;
  }

  .hero {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    margin-bottom: 24px;
  }

  .eyebrow {
    color: #38bdf8;
    font-size: 0.78rem;
    font-weight: 700;
    letter-spacing: 0.16em;
    margin: 0 0 8px;
    text-transform: uppercase;
  }

  h1,
  h2,
  p {
    margin: 0;
  }

  h1 {
    font-size: clamp(2rem, 5vw, 4rem);
    line-height: 1;
  }

  .summary {
    color: #94a3b8;
    margin-top: 12px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 18px;
  }

  .card {
    background: #111c33;
    border: 1px solid #23314f;
    border-radius: 20px;
    box-shadow: 0 18px 60px rgb(0 0 0 / 0.25);
    padding: 20px;
  }

  header {
    align-items: flex-start;
    display: flex;
    justify-content: space-between;
    gap: 16px;
  }

  header p,
  dt {
    color: #94a3b8;
  }

  span.online,
  span.stale,
  span.offline {
    border-radius: 999px;
    font-size: 0.78rem;
    font-weight: 700;
    padding: 6px 10px;
    text-transform: uppercase;
  }

  .online {
    background: #064e3b;
    color: #6ee7b7;
  }

  .stale {
    background: #713f12;
    color: #fde68a;
  }

  .offline {
    background: #450a0a;
    color: #fecaca;
  }

  dl {
    display: grid;
    gap: 12px;
    margin: 20px 0;
  }

  dl div {
    background: #0f172a;
    border-radius: 14px;
    padding: 12px;
  }

  dt {
    font-size: 0.8rem;
    margin-bottom: 4px;
  }

  dd {
    font-size: 1rem;
    font-weight: 700;
    margin: 0;
  }

  .controls {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  button {
    background: #2563eb;
    border: 0;
    border-radius: 12px;
    color: white;
    cursor: pointer;
    font-weight: 700;
    padding: 10px 12px;
  }

  button span {
    display: block;
    font-size: 1.2rem;
  }

  button:hover:not(:disabled) {
    background: #1d4ed8;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }

  .secondary {
    background: #334155;
  }

  .danger {
    background: #dc2626;
  }

  .error,
  .empty {
    background: #111c33;
    border: 1px solid #23314f;
    border-radius: 16px;
    padding: 18px;
  }

  .error {
    border-color: #b91c1c;
    color: #fecaca;
    margin-bottom: 16px;
  }
</style>
