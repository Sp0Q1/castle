export interface Slice {
  label: string;
  value: number;
  color: string;
}

const SIZE = 160;
const R = 70;
const C = SIZE / 2;

function point(angle: number): [number, number] {
  return [C + R * Math.cos(angle), C + R * Math.sin(angle)];
}

export function PieChart({
  title,
  slices,
}: {
  title: string;
  slices: Slice[];
}) {
  const data = slices.filter((s) => s.value > 0);
  const total = data.reduce((sum, s) => sum + s.value, 0);

  let angle = -Math.PI / 2;
  const arcs = data.map((s) => {
    const fraction = s.value / total;
    const start = angle;
    const end = angle + fraction * 2 * Math.PI;
    angle = end;
    return { ...s, start, end, fraction };
  });

  return (
    <div className="card pie">
      <h3>{title}</h3>
      {total === 0 ? (
        <p className="muted">No findings yet.</p>
      ) : (
        <div className="pie-body">
          <svg
            viewBox={`0 0 ${SIZE} ${SIZE}`}
            width={SIZE}
            height={SIZE}
            role="img"
            aria-label={title}
          >
            {arcs.map((a) => {
              if (a.fraction >= 1) {
                return (
                  <circle key={a.label} cx={C} cy={C} r={R} fill={a.color} />
                );
              }
              const [x1, y1] = point(a.start);
              const [x2, y2] = point(a.end);
              const large = a.end - a.start > Math.PI ? 1 : 0;
              const d = `M ${C} ${C} L ${x1} ${y1} A ${R} ${R} 0 ${large} 1 ${x2} ${y2} Z`;
              return <path key={a.label} d={d} fill={a.color} />;
            })}
          </svg>
          <ul className="pie-legend">
            {arcs.map((a) => (
              <li key={a.label}>
                <span className="swatch" style={{ background: a.color }} />
                <span className="pie-label">{a.label}</span>
                <span className="muted">{a.value}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
