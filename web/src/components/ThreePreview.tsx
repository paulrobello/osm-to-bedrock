'use client';

import { useEffect, useMemo, useRef } from 'react';
import { Canvas, useThree } from '@react-three/fiber';
import { OrbitControls } from '@react-three/drei';
import * as THREE from 'three';
import { BLOCK_COLORS } from '@/lib/blockColors';
import type { PreviewBlock, PreviewBounds, PreviewSpawn } from '@/hooks/usePreview';

interface ThreePreviewProps {
  blocks: PreviewBlock[];
  bounds: PreviewBounds;
  spawn: PreviewSpawn | null;
}

function BlockInstances({
  positions,
  color,
  blockSize,
}: {
  positions: [number, number, number][];
  color: string;
  blockSize: number;
}) {
  const meshRef = useRef<THREE.InstancedMesh>(null);

  useEffect(() => {
    if (!meshRef.current || positions.length === 0) return;
    const matrix = new THREE.Matrix4();
    const scale = new THREE.Vector3(blockSize, blockSize, blockSize);
    positions.forEach((pos, i) => {
      matrix.compose(
        new THREE.Vector3(pos[0], pos[1], pos[2]),
        new THREE.Quaternion(),
        scale,
      );
      meshRef.current!.setMatrixAt(i, matrix);
    });
    meshRef.current.instanceMatrix.needsUpdate = true;
  }, [positions, blockSize]);

  if (positions.length === 0) return null;

  return (
    <instancedMesh ref={meshRef} args={[undefined, undefined, positions.length]}>
      <boxGeometry args={[1, 1, 1]} />
      <meshLambertMaterial color={color} />
    </instancedMesh>
  );
}

function CameraSetup({ viewSize, target }: { viewSize: number; target: [number, number, number] }) {
  const { camera, size } = useThree();

  useEffect(() => {
    const dist = viewSize * 0.6;
    camera.position.set(target[0] + dist, target[1] + dist * 0.5, target[2] + dist);
    camera.lookAt(target[0], target[1], target[2]);
    if ('zoom' in camera) {
      // Scale zoom so the scene fills ~80% of the viewport
      const ortho = camera as THREE.OrthographicCamera;
      const viewportSpan = Math.min(size.width, size.height);
      // eslint-disable-next-line react-hooks/immutability -- R3F camera is intentionally mutated imperatively
      ortho.zoom = Math.max(1, viewportSpan / (viewSize * 1.8));
      camera.updateProjectionMatrix();
    }
  }, [camera, viewSize, target, size]);

  return null;
}

// Priority offsets to prevent z-fighting for blocks at the same Y level.
// Higher priority blocks render on top.
const TYPE_Y_OFFSET: Record<string, number> = {
  GrassBlock: 0,
  Dirt: 0,
  Sand: 0,
  Stone: 0,
  OakLog: 0,
  Water: -0.15,
  BlackConcrete: 0.1,
  GrayConcrete: 0.1,
  Concrete: 0.1,
  Gravel: 0.05,
  StoneSlab: 0.12,
  YellowConcrete: 0.15,
  WhiteConcrete: 0.15,
  StoneBrick: 0.2,
  Brick: 0.2,
  OakPlanks: 0.2,
  SprucePlanks: 0.2,
  Sandstone: 0.2,
  Rail: 0.1,
};

function Scene({ blocks, bounds, spawn }: ThreePreviewProps) {
  const rangeX = bounds.max_x - bounds.min_x;
  const rangeZ = bounds.max_z - bounds.min_z;
  const range = Math.max(rangeX, rangeZ, 10);

  // Normalize XZ to fit ~200 unit box
  const scaleFactor = 200 / range;
  // Exaggerate Y axis so height differences are visible (3x relative to XZ)
  const yScale = scaleFactor * 3;
  const blockSize = Math.max(scaleFactor, 0.5);

  const cx = (bounds.min_x + bounds.max_x) / 2;
  const cz = (bounds.min_z + bounds.max_z) / 2;
  const minY = blocks.length > 0 ? blocks.reduce((m, b) => Math.min(m, b.y), Infinity) : 0;

  // Camera target: spawn point (normalized) or center
  const camTarget: [number, number, number] = spawn
    ? [(spawn.x - cx) * scaleFactor, (spawn.y - minY) * yScale, (spawn.z - cz) * scaleFactor]
    : [0, 0, 0];

  const blocksByType = useMemo(() => {
    const map = new Map<string, [number, number, number][]>();
    for (const block of blocks) {
      if (!map.has(block.type)) {
        map.set(block.type, []);
      }
      const yOffset = TYPE_Y_OFFSET[block.type] ?? 0;
      map.get(block.type)!.push([
        (block.x - cx) * scaleFactor,
        (block.y - minY) * yScale + yOffset,
        (block.z - cz) * scaleFactor,
      ]);
    }
    return map;
  }, [blocks, cx, cz, minY, scaleFactor, yScale]);

  return (
    <>
      <CameraSetup viewSize={200} target={camTarget} />
      <ambientLight intensity={0.8} />
      <directionalLight position={[100, 200, 100]} intensity={0.8} />
      <directionalLight position={[-100, 50, -100]} intensity={0.3} />
      {Array.from(blocksByType.entries()).map(([type, positions]) => {
        const color = BLOCK_COLORS[type] || '#888888';
        return <BlockInstances key={type} positions={positions} color={color} blockSize={blockSize} />;
      })}
      <OrbitControls makeDefault target={camTarget} />
    </>
  );
}

export function ThreePreview({ blocks, bounds, spawn }: ThreePreviewProps) {
  if (blocks.length === 0) {
    return (
      <div
        className="flex h-full w-full items-center justify-center"
        style={{ background: '#0e1018', color: 'var(--text-muted)' }}
      >
        No preview data available
      </div>
    );
  }

  return (
    <div className="h-full w-full" style={{ background: '#0e1018' }}>
      <Canvas
        orthographic
        camera={{
          zoom: 2,
          position: [200, 150, 200],
          near: -5000,
          far: 5000,
        }}
        scene={{ background: new THREE.Color('#0e1018') }}
      >
        <Scene blocks={blocks} bounds={bounds} spawn={spawn} />
      </Canvas>
    </div>
  );
}
