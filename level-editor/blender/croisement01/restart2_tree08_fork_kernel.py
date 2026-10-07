"""Bounded CPU diagnostic of isolated forks with the installed VTK kernel."""
import json
from pathlib import Path

import numpy as np
from vtkmodules.vtkCommonCore import vtkPoints, vtkSMPTools
from vtkmodules.vtkCommonDataModel import vtkCellArray, vtkPolyData
from vtkmodules.vtkFiltersCore import vtkCleanPolyData, vtkTriangleFilter
from vtkmodules.vtkFiltersGeneral import vtkBooleanOperationPolyDataFilter
from vtkmodules.util.numpy_support import numpy_to_vtk, vtk_to_numpy

from restart2_tree08_local_fork import audit


def poly(vertices, faces):
    points = vtkPoints()
    points.SetData(numpy_to_vtk(np.asarray(vertices, dtype=np.float64), deep=True))
    cells = vtkCellArray()
    for face in faces:
        cells.InsertNextCell(3)
        for index in face:
            cells.InsertCellPoint(int(index))
    result = vtkPolyData()
    result.SetPoints(points)
    result.SetPolys(cells)
    return result


def main():
    vtkSMPTools.Initialize(2)
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    source = root / 'tree08-v12-local-fork-cpu-v1'
    data = np.load(source / 'minimal-forks.npz')
    origin = np.array([488., -667., 230.])
    output = root / 'tree08-v12-local-fork-kernel-v1'
    output.mkdir(exist_ok=False)
    records = []
    # Each pair is independent. These are diagnostic outputs, never a model.
    for index in [29, 93, 33, 96]:
        operation = vtkBooleanOperationPolyDataFilter()
        operation.SetOperationToUnion()
        operation.SetTolerance(1e-8)
        operation.SetInputData(0, poly(data['continuation_vertices'] - origin, data['continuation_faces']))
        operation.SetInputData(1, poly(data[f'vertices_{index}'] - origin, data[f'faces_{index}']))
        print('PAIR', index, flush=True)
        operation.Update()
        clean = vtkCleanPolyData()
        clean.SetInputConnection(operation.GetOutputPort())
        clean.ToleranceIsAbsoluteOn()
        clean.SetAbsoluteTolerance(1e-9)
        clean.ConvertPolysToLinesOff()
        clean.ConvertLinesToPointsOff()
        clean.Update()
        triangles = vtkTriangleFilter()
        triangles.SetInputConnection(clean.GetOutputPort())
        triangles.Update()
        result = triangles.GetOutput()
        if not result.GetNumberOfPoints():
            records.append(dict(section=index, status='EMPTY'))
            continue
        vertices = vtk_to_numpy(result.GetPoints().GetData())
        faces = vtk_to_numpy(result.GetPolys().GetConnectivityArray()).reshape(-1, 3)
        check = audit(vertices, faces)
        np.savez_compressed(output / f'pair-{index}.npz', vertices=vertices + origin, faces=faces)
        records.append(dict(section=index, topology=check, vertices=len(vertices), faces=len(faces)))
        (output / 'report.json').write_text(json.dumps(dict(status='DIAGNOSTIC; no source/displacement acceptance', pairs=records), indent=2) + '\n')
        print(records[-1], flush=True)


if __name__ == '__main__':
    main()
