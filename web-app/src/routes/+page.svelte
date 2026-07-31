<script lang="ts">
  import { onMount } from 'svelte';
  import {
    backendWsUrl,
    deleteRobot,
    fetchRobots,
    sendRobotCommand,
    sendSimulatedEvent
  } from '$lib/api';
  import type { Robot, RobotStreamMessage } from '$lib/types';

  type MoveForm = {
    target_position_x: string;
    target_position_y: string;
    set_velocity: string;
  };

  const emptyMoveForm = (): MoveForm => ({
    target_position_x: '',
    target_position_y: '',
    set_velocity: '1'
  });

  let robots: Robot[] = [];
  let loading = true;
  let error = '';
  let pendingAction = '';
  let moveForms: Record<string, MoveForm> = {};

  onMount(() => {
    void loadRobots();
    const socket = new WebSocket(`${backendWsUrl()}/robots/stream`);
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data) as RobotStreamMessage;
      if (message.event_type === 'robot_deleted' && message.robot_id) {
        robots = robots.filter((robot) => robot.robot_id !== message.robot_id);
        return;
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
      moveForms = Object.fromEntries(
        robots.map((robot) => [robot.robot_id, moveForms[robot.robot_id] ?? formFromRobot(robot)])
      );
    } catch (err) {
      error = err instanceof Error ? err.message : 'Failed to load robots.';
    } finally {
      loading = false;
    }
  }

  function formFromRobot(robot: Robot): MoveForm {
    return {
      target_position_x: robot.target_position_x?.toString() ?? '',
      target_position_y: robot.target_position_y?.toString() ?? '',
      set_velocity: robot.set_velocity?.toString() ?? '1'
    };
  }

  function upsertRobot(robot: Robot) {
    moveForms = {
      ...moveForms,
      [robot.robot_id]: moveForms[robot.robot_id] ?? formFromRobot(robot)
    };
    const index = robots.findIndex((item) => item.robot_id === robot.robot_id);
    if (index === -1) {
      robots = [...robots, robot].sort((left, right) => left.robot_id.localeCompare(right.robot_id));
      return;
    }
    robots = robots.map((item) => (item.robot_id === robot.robot_id ? robot : item));
  }

  function moveFormFor(robotId: string) {
    return moveForms[robotId] ?? emptyMoveForm();
  }

  function updateMoveForm(robotId: string, field: keyof MoveForm, value: string) {
    moveForms = {
      ...moveForms,
      [robotId]: {
        ...moveFormFor(robotId),
        [field]: value
      }
    };
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

  async function sendSetVelocity(robot: Robot) {
    const form = moveFormFor(robot.robot_id);
    if (form.set_velocity.trim() === '') {
      error = 'Enter set velocity.';
      return;
    }
    const set_velocity = Number(form.set_velocity);

    if (!Number.isFinite(set_velocity) || set_velocity < 0.01 || set_velocity > 10) {
      error = 'Enter a set velocity between 0.01 and 10.0.';
      return;
    }

    await runCommand(robot, 'set_velocity', { set_velocity });
  }

  async function sendMove(robot: Robot) {
    const form = moveFormFor(robot.robot_id);
    if (
      form.target_position_x.trim() === '' ||
      form.target_position_y.trim() === '' ||
      form.set_velocity.trim() === ''
    ) {
      error = 'Enter target X, target Y and set velocity.';
      return;
    }

    const target_position_x = Number(form.target_position_x);
    const target_position_y = Number(form.target_position_y);
    const set_velocity = Number(form.set_velocity);

    if (
      ![target_position_x, target_position_y, set_velocity].every(Number.isFinite) ||
      set_velocity < 0.01 ||
      set_velocity > 10
    ) {
      error = 'Enter valid target coordinates and a set velocity between 0.01 and 10.0.';
      return;
    }

    await runCommand(robot, 'set_velocity', { set_velocity });
    await runCommand(robot, 'move', { target_position_x, target_position_y });
  }

  async function toggleStop(robot: Robot) {
    await runCommand(robot, 'stop', { stop: !robot.stop });
  }

  async function simulateEvent(robot: Robot, eventType: 'extreme_temperature' | 'robot_stack') {
    pendingAction = `${robot.robot_id}:${eventType}`;
    error = '';
    try {
      await sendSimulatedEvent(robot.robot_id, eventType, { simulated: true });
    } catch (err) {
      error = err instanceof Error ? err.message : `Failed to send ${eventType}.`;
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

  function velocity(value: number | null) {
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
              <dt>Status</dt>
              <dd>{robot.status}</dd>
            </div>
            <div>
              <dt>Battery</dt>
              <dd>{robot.battery_level.toFixed(1)}%</dd>
            </div>
            <div>
              <dt>Current position</dt>
              <dd>x {coordinate(robot.position_x)} cm, y {coordinate(robot.position_y)} cm</dd>
            </div>
            <div>
              <dt>Current velocity</dt>
              <dd>{velocity(robot.velocity)}</dd>
            </div>
            <div>
              <dt>Target position</dt>
              <dd>x {coordinate(robot.target_position_x)} cm, y {coordinate(robot.target_position_y)} cm</dd>
            </div>
            <div>
              <dt>Set velocity</dt>
              <dd>{velocity(robot.set_velocity)}</dd>
            </div>
            <div>
              <dt>Direction</dt>
              <dd>{heading(robot.direction_degrees)}</dd>
            </div>
            <div>
              <dt>Stopped</dt>
              <dd>{robot.stop ? 'yes' : 'no'}</dd>
            </div>
            <div>
              <dt>Robot state</dt>
              <dd>{robot.state}</dd>
            </div>
            <div>
              <dt>Current command</dt>
              <dd>{robot.current_command ?? 'none'}</dd>
            </div>
            <div>
              <dt>Current mission</dt>
              <dd>{robot.current_mission ?? 'none'}</dd>
            </div>
            <div>
              <dt>Command status</dt>
              <dd>{robot.current_command_status ?? 'idle'}</dd>
            </div>
          </dl>

          <div class="move-form" aria-label={`Move controls for ${robot.name}`}>
            <div class="move-row move-row--target">
              <label>
                <span>Target X</span>
                <input
                  type="number"
                  step="any"
                  value={moveFormFor(robot.robot_id).target_position_x}
                  on:input={(event) => updateMoveForm(robot.robot_id, 'target_position_x', (event.currentTarget as HTMLInputElement).value)}
                />
              </label>
              <label>
                <span>Target Y</span>
                <input
                  type="number"
                  step="any"
                  value={moveFormFor(robot.robot_id).target_position_y}
                  on:input={(event) => updateMoveForm(robot.robot_id, 'target_position_y', (event.currentTarget as HTMLInputElement).value)}
                />
              </label>
              <button disabled={pendingAction !== ''} on:click={() => sendMove(robot)}>Move</button>
            </div>
            <div class="move-row move-row--velocity">
              <label>
                <span>Set velocity</span>
                <input
                  type="number"
                  step="any"
                  min="0.01"
                  value={moveFormFor(robot.robot_id).set_velocity}
                  on:input={(event) => updateMoveForm(robot.robot_id, 'set_velocity', (event.currentTarget as HTMLInputElement).value)}
                />
              </label>
              <button disabled={pendingAction !== ''} on:click={() => sendSetVelocity(robot)}>Set velocity</button>
            </div>
          </div>

          <div class="controls" aria-label={`Controls for ${robot.name}`}>
            <button
              disabled={pendingAction !== ''}
              on:click={() => toggleStop(robot)}
            >
              <span>{robot.stop ? '▶' : '■'}</span>
              {robot.stop ? 'Resume' : 'Stop'}
            </button>
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

          <section class="event-controls" aria-label={`Simulate events for ${robot.name}`}>
            <h3>Simulate events</h3>
            <button
              class="danger"
              disabled={pendingAction !== ''}
              on:click={() => simulateEvent(robot, 'extreme_temperature')}
            >
              Extreme temperature
            </button>
            <button
              class="warning"
              disabled={pendingAction !== ''}
              on:click={() => simulateEvent(robot, 'robot_stack')}
            >
              Robot stack
            </button>
          </section>
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
      Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
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
  h3,
  p {
    margin: 0;
  }

  h1 {
    font-size: clamp(2rem, 5vw, 4rem);
    line-height: 1;
  }

  h3 {
    color: #cbd5e1;
    font-size: 0.95rem;
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
    gap: 12px;
  }

  header p,
  .empty,
  .error {
    color: #94a3b8;
  }

  .error {
    background: rgb(127 29 29 / 0.5);
    border: 1px solid rgb(248 113 113 / 0.4);
    border-radius: 14px;
    margin-bottom: 16px;
    padding: 12px 14px;
  }

  .empty {
    padding: 24px 0;
  }

  span.online,
  span.stale,
  span.offline {
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    padding: 6px 10px;
    text-transform: uppercase;
  }

  span.online {
    background: rgb(22 163 74 / 0.22);
    color: #4ade80;
  }

  span.stale {
    background: rgb(202 138 4 / 0.2);
    color: #fbbf24;
  }

  span.offline {
    background: rgb(71 85 105 / 0.45);
    color: #cbd5e1;
  }

  dl {
    display: grid;
    gap: 10px;
    margin: 18px 0 20px;
  }

  dl div {
    display: flex;
    justify-content: space-between;
    gap: 16px;
  }

  dt {
    color: #94a3b8;
  }

  dd {
    margin: 0;
    text-align: right;
  }

  .move-form {
    border-top: 1px solid #23314f;
    display: grid;
    gap: 12px;
    margin-top: 16px;
    padding-top: 16px;
  }

  .move-row {
    display: grid;
    gap: 12px;
  }

  .move-row--target {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) auto;
  }

  .move-row--velocity {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .move-form label {
    display: grid;
    gap: 6px;
  }

  .move-form span {
    color: #94a3b8;
    font-size: 0.86rem;
  }

  input {
    background: #0f172a;
    border: 1px solid #334155;
    border-radius: 10px;
    color: inherit;
    padding: 10px 12px;
    min-width: 0;
  }

  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 14px;
  }

  .event-controls {
    border-top: 1px solid #23314f;
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 14px;
    padding-top: 14px;
  }

  .event-controls h3 {
    flex-basis: 100%;
  }

  button {
    align-items: center;
    background: #2563eb;
    border: 0;
    border-radius: 12px;
    color: white;
    cursor: pointer;
    display: inline-flex;
    gap: 8px;
    padding: 10px 14px;
  }

  button.secondary {
    background: #1e293b;
  }

  button.danger {
    background: #b91c1c;
  }

  button.warning {
    background: #ca8a04;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }

  @media (max-width: 720px) {
    .move-row--target,
    .move-row--velocity {
      grid-template-columns: 1fr;
    }
  }
</style>
