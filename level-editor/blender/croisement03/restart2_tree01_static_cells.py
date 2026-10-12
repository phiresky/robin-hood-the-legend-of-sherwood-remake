"""Embed directly reviewed static leaf proposal separately from dynamic source cells."""
import restart2_tree01_native_cells as cells

def main():
 cells.SOURCE=cells.B/'tree01-static-leaf-proposal-v1';cells.FRAME='proposed-source-rgba.png';cells.UNION='candidate.png';cells.SOURCE_KIND='proposed-static-native-samples';cells.O=cells.B/'tree01-static-cells-cpu-v3';cells.main()
if __name__=='__main__':main()
