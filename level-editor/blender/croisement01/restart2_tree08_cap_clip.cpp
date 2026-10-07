// Bounded planar source-cap subtraction with the installed polygon headers.
#include <boost/geometry.hpp>
#include <iomanip>
#include <iostream>
#include <stdexcept>
#include <cmath>
namespace bg = boost::geometry;
using Point = bg::model::d2::point_xy<double>;
using Polygon = bg::model::polygon<Point>;
using Multi = bg::model::multi_polygon<Polygon>;
template<class Ring> void identify_duplicates(Ring &ring) {
  Ring cleaned;
  for (const auto &point : ring) {
    if (cleaned.empty() || std::hypot(bg::get<0>(point)-bg::get<0>(cleaned.back()),
                                     bg::get<1>(point)-bg::get<1>(cleaned.back())) > 1e-10)
      cleaned.push_back(point);
  }
  if (!cleaned.empty() && !bg::equals(cleaned.front(), cleaned.back())) cleaned.push_back(cleaned.front());
  ring = cleaned;
}
Multi read_paths() {
  size_t count;
  if (!(std::cin >> count) || count > 256) throw std::runtime_error("path count");
  Multi paths;
  paths.resize(count);
  for (auto &path : paths) {
    size_t points;
    if (!(std::cin >> points) || points > 10000) throw std::runtime_error("point count");
    for (size_t i = 0; i < points; ++i) {
      double x,y;
      if (!(std::cin >> x >> y)) throw std::runtime_error("point input");
      path.outer().emplace_back(x,y);
    }
    bg::correct(path);
    if (!bg::is_valid(path)) throw std::runtime_error("invalid polygon");
  }
  return paths;
}
int main() {
  auto remaining = read_paths();
  const auto clips = read_paths();
  for (const auto &clip : clips) {
    Multi next;
    bg::difference(remaining, clip, next);
    remaining = next;
  }
  size_t count = 0;
  for (const auto &p : remaining) count += 1+p.inners().size();
  std::cout << std::setprecision(17) << count << '\n';
  const auto write = [](const auto &ring) {
    std::cout << ring.size()-1;
    for (size_t i=0; i+1<ring.size(); ++i)
      std::cout << ' ' << bg::get<0>(ring[i]) << ' ' << bg::get<1>(ring[i]);
    std::cout << '\n';
  };
  for (auto &p : remaining) {
    identify_duplicates(p.outer());
    for (auto &ring : p.inners()) identify_duplicates(ring);
    bg::remove_spikes(p);
    std::string validity;
    if (!bg::is_valid(p, validity)) {
      std::cerr << "Invalid difference: " << validity << '\n' << std::setprecision(17) << bg::wkt(p) << '\n';
      throw std::runtime_error("invalid difference");
    }
    write(p.outer());
    for (const auto &ring : p.inners()) write(ring);
  }
}
