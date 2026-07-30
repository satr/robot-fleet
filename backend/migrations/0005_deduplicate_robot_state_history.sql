WITH ordered_states AS (
    SELECT
        robot_id,
        recorded_at,
        state,
        LAG(state) OVER (PARTITION BY robot_id ORDER BY recorded_at) AS previous_state
    FROM robot_state_history
),
duplicate_states AS (
    SELECT robot_id, recorded_at
    FROM ordered_states
    WHERE previous_state = state
)
DELETE FROM robot_state_history history
USING duplicate_states duplicates
WHERE history.robot_id = duplicates.robot_id
  AND history.recorded_at = duplicates.recorded_at;
