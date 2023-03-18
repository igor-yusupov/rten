///! Geometry functions for pre and post-processing images.
///!
///! TODO: Move these out of Wasnn and into a separate crate.
use std::fmt;
use std::fmt::Display;
use std::iter::zip;
use std::ops::Range;
use std::slice::Iter;

use crate::tensor::{MatrixLayout, NdTensor, NdTensorView, NdTensorViewMut};

pub type Coord = i32;

/// A point defined by integer X and Y coordinates.
#[derive(Copy, Clone, Default, Eq, PartialEq)]
pub struct Point {
    pub x: Coord,
    pub y: Coord,
}

impl Point {
    pub fn from_yx(y: Coord, x: Coord) -> Point {
        Point { y, x }
    }

    /// Return self as a [y, x] index.
    pub fn coord(self) -> [usize; 2] {
        // FIXME - Handle case where these coords are out-of-bounds.
        [self.y as usize, self.x as usize]
    }

    pub fn translate(self, y: Coord, x: Coord) -> Point {
        Point {
            y: self.y + y,
            x: self.x + x,
        }
    }

    /// Return the neighbors of the current point in clockwise order, starting
    /// from the point directly above `self`.
    pub fn neighbors(self) -> [Point; 8] {
        [
            self.translate(-1, 0),  // N
            self.translate(-1, 1),  // NE
            self.translate(0, 1),   // E
            self.translate(1, 1),   // SE
            self.translate(1, 0),   // S
            self.translate(1, -1),  // SW
            self.translate(0, -1),  // W
            self.translate(-1, -1), // NW
        ]
    }

    /// Return the euclidean distance between this point and another point.
    pub fn distance(self, other: Point) -> f32 {
        Vec2::from_points(self, other).length()
    }
}

impl fmt::Debug for Point {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({}, {})", self.y, self.x)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub fn from_yx(y: f32, x: f32) -> Vec2 {
        Vec2 { y, x }
    }

    /// Return the vector from `start` to `end`.
    pub fn from_points(start: Point, end: Point) -> Vec2 {
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        Vec2::from_yx(dy as f32, dx as f32)
    }

    pub fn length(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Return the magnitude of the cross product of this vector with `other`.
    pub fn cross_product_norm(&self, other: Vec2) -> f32 {
        self.x * other.y - self.y * other.x
    }

    /// Return the dot product of this vector with `other`.
    pub fn dot(&self, other: Vec2) -> f32 {
        self.x * other.x + self.y * other.y
    }

    /// Return a copy of this vector scaled such that the length is 1.
    pub fn normalized(&self) -> Vec2 {
        let inv_len = 1. / self.length();
        Vec2::from_yx(self.y * inv_len, self.x * inv_len)
    }

    /// Return a vector perpendicular to this vector.
    pub fn perpendicular(&self) -> Vec2 {
        Vec2 {
            y: -self.x,
            x: self.y,
        }
    }
}

impl std::ops::Add<Vec2> for Vec2 {
    type Output = Vec2;

    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2 {
            y: self.y + rhs.y,
            x: self.x + rhs.x,
        }
    }
}

impl std::ops::Neg for Vec2 {
    type Output = Vec2;

    fn neg(self) -> Vec2 {
        Vec2 {
            y: -self.y,
            x: -self.x,
        }
    }
}

impl std::ops::Mul<f32> for Vec2 {
    type Output = Vec2;

    fn mul(self, rhs: f32) -> Vec2 {
        Vec2 {
            y: self.y * rhs,
            x: self.x * rhs,
        }
    }
}

impl std::ops::Sub<f32> for Vec2 {
    type Output = Vec2;

    fn sub(self, rhs: f32) -> Vec2 {
        Vec2 {
            y: self.y - rhs,
            x: self.x - rhs,
        }
    }
}

impl std::ops::Sub<Vec2> for Vec2 {
    type Output = Vec2;

    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2 {
            y: self.y - rhs.y,
            x: self.x - rhs.x,
        }
    }
}

fn sort_pair<T: Ord>(pair: (T, T)) -> (T, T) {
    if pair.0 <= pair.1 {
        pair
    } else {
        (pair.1, pair.0)
    }
}

/// Compute the overlap between two 1D lines `a` and `b`, where each line is
/// given as (start, end) coords.
fn overlap(a: (i32, i32), b: (i32, i32)) -> i32 {
    let a = sort_pair(a);
    let b = sort_pair(b);
    let ((a_start, a_end), (b_start, b_end)) = sort_pair((a, b));
    (a_end - b_start).clamp(0, b_end - b_start)
}

/// A bounded line segment defined by a start and end point.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Line {
    pub start: Point,
    pub end: Point,
}

impl Line {
    pub fn from_endpoints(start: Point, end: Point) -> Line {
        Line { start, end }
    }

    /// Return true if this line has zero length.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Return the euclidean distance between a point and the closest coordinate
    /// that lies on the line.
    pub fn distance(&self, p: Point) -> f32 {
        if self.is_empty() {
            return self.start.distance(p);
        }

        // Method taken from http://www.faqs.org/faqs/graphics/algorithms-faq/,
        // "Subject 1.02: How do I find the distance from a point to a line?".

        // Compute normalized scalar projection of line from `start` to `p` onto
        // self. This indicates how far along the `self` line the nearest point
        // to `p` is.
        let ab = Vec2::from_points(self.start, self.end);
        let ac = Vec2::from_points(self.start, p);
        let scalar_proj = ac.dot(ab) / (ab.length() * ab.length());

        if scalar_proj <= 0. {
            // Nearest point is start of line.
            self.start.distance(p)
        } else if scalar_proj >= 1. {
            // Nearest point is end of line.
            self.end.distance(p)
        } else {
            let start_x = self.start.x as f32;
            let start_y = self.start.y as f32;
            let intercept_x = start_x + ab.x * scalar_proj;
            let intercept_y = start_y + ab.y * scalar_proj;
            let proj_line = Vec2::from_yx(intercept_y - p.y as f32, intercept_x - p.x as f32);
            proj_line.length()
        }
    }

    /// Return the number of pixels by which this line overlaps `other` in the
    /// vertical direction.
    pub fn vertical_overlap(&self, other: Line) -> i32 {
        overlap((self.start.y, self.end.y), (other.start.y, other.end.y))
    }

    /// Return the number of pixels by which this line overlaps `other` in the
    /// horizontal direction.
    pub fn horizontal_overlap(&self, other: Line) -> i32 {
        overlap((self.start.x, self.end.x), (other.start.x, other.end.x))
    }
}

/// Rectangle defined by left, top, right and bottom integer coordinates.
///
/// The left and top coordinates are inclusive. The right and bottom coordinates
/// are exclusive.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Rect {
    top_left: Point,
    bottom_right: Point,
}

impl Rect {
    /// Return a rect with top-left corner at 0, 0 and the given height/width.
    pub fn from_hw(height: Coord, width: Coord) -> Rect {
        Self::new(Point::default(), Point::from_yx(height, width))
    }

    /// Return a rect with the given top, left, bottom and right coordinates.
    pub fn from_tlbr(top: Coord, left: Coord, bottom: Coord, right: Coord) -> Rect {
        Self::new(Point::from_yx(top, left), Point::from_yx(bottom, right))
    }

    pub fn new(top_left: Point, bottom_right: Point) -> Rect {
        Rect {
            top_left,
            bottom_right,
        }
    }

    pub fn area(&self) -> Coord {
        self.width() * self.height()
    }

    pub fn width(&self) -> Coord {
        // TODO - Handle inverted rects here
        self.bottom_right.x - self.top_left.x
    }

    pub fn height(&self) -> Coord {
        // TODO - Handle inverted rects here
        self.bottom_right.y - self.top_left.y
    }

    pub fn top(&self) -> Coord {
        self.top_left.y
    }

    pub fn left(&self) -> Coord {
        self.top_left.x
    }

    pub fn right(&self) -> Coord {
        self.bottom_right.x
    }

    pub fn bottom(&self) -> Coord {
        self.bottom_right.y
    }

    /// Return true if the width or height of this rect are <= 0.
    pub fn is_empty(&self) -> bool {
        self.right() <= self.left() || self.bottom() <= self.top()
    }

    /// Return the center point of the rect.
    pub fn center(&self) -> Point {
        let y = (self.top_left.y + self.bottom_right.y) / 2;
        let x = (self.top_left.x + self.bottom_right.x) / 2;
        Point::from_yx(y, x)
    }

    /// Return the coordinate of the top-left corner of the rect.
    pub fn top_left(&self) -> Point {
        self.top_left
    }

    /// Return the coordinate of the top-right corner of the rect.
    pub fn top_right(&self) -> Point {
        Point::from_yx(self.top_left.y, self.bottom_right.x)
    }

    /// Return the coordinate of the bottom-left corner of the rect.
    pub fn bottom_left(&self) -> Point {
        Point::from_yx(self.bottom_right.y, self.top_left.x)
    }

    /// Return the coordinate of the bottom-right corner of the rect.
    pub fn bottom_right(&self) -> Point {
        self.bottom_right
    }

    /// Return the line segment of the left edge of the rect.
    pub fn left_edge(&self) -> Line {
        Line::from_endpoints(self.top_left(), self.bottom_left())
    }

    /// Return the line segment of the top edge of the rect.
    pub fn top_edge(&self) -> Line {
        Line::from_endpoints(self.top_left(), self.top_right())
    }

    /// Return the line segment of the right edge of the rect.
    pub fn right_edge(&self) -> Line {
        Line::from_endpoints(self.top_right(), self.bottom_right())
    }

    /// Return the line segment of the bottom edge of the rect.
    pub fn bottom_edge(&self) -> Line {
        Line::from_endpoints(self.bottom_left(), self.bottom_right())
    }

    /// Return the top, left, bottom and right coordinates as an array.
    pub fn tlbr(&self) -> [Coord; 4] {
        [
            self.top_left.y,
            self.top_left.x,
            self.bottom_right.y,
            self.bottom_right.x,
        ]
    }

    /// Return a new Rect with each coordinate adjusted by an offset.
    pub fn adjust_tlbr(&self, top: Coord, left: Coord, bottom: Coord, right: Coord) -> Rect {
        Rect {
            top_left: self.top_left.translate(top, left),
            bottom_right: self.bottom_right.translate(bottom, right),
        }
    }

    /// Return a new with each side adjusted so that the result lies inside
    /// `rect`.
    pub fn clamp(&self, rect: Rect) -> Rect {
        let top = self.top().max(rect.top());
        let left = self.left().max(rect.left());
        let bottom = self.bottom().min(rect.bottom());
        let right = self.right().min(rect.right());
        Rect {
            top_left: Point::from_yx(top, left),
            bottom_right: Point::from_yx(bottom, right),
        }
    }

    /// Return true if the intersection of this rect and `other` is non-empty.
    pub fn intersects(&self, other: Rect) -> bool {
        self.left_edge().vertical_overlap(other.left_edge()) > 0
            && self.top_edge().horizontal_overlap(other.top_edge()) > 0
    }

    /// Return the smallest rect that contains both this rect and `other`.
    pub fn union(&self, other: Rect) -> Rect {
        let t = self.top().min(other.top());
        let l = self.left().min(other.left());
        let b = self.bottom().max(other.bottom());
        let r = self.right().max(other.right());
        Rect::from_tlbr(t, l, b, r)
    }

    /// Return true if `other` lies entirely within this rect.
    pub fn contains(&self, other: Rect) -> bool {
        self.union(other) == *self
    }
}

/// A collection of polygons, where each polygon is defined by a slice of points.
///
/// `Polygons` is primarily useful when building up collections of many polygons
/// as it stores all points in a single Vec, which is more efficient than
/// allocating a separate Vec for each polygon's points.
pub struct Polygons {
    points: Vec<Point>,

    // Offsets within `points` where each polygon starts and ends.
    polygons: Vec<Range<usize>>,
}

impl Polygons {
    /// Construct an empty polygon collection.
    pub fn new() -> Polygons {
        Polygons {
            points: Vec::new(),
            polygons: Vec::new(),
        }
    }

    /// Add a new polygon to the list, defined by the given points.
    pub fn push(&mut self, points: &[Point]) {
        let range = self.points.len()..self.points.len() + points.len();
        self.polygons.push(range);
        self.points.extend_from_slice(points);
    }

    /// Return the number of polygons in the collection.
    pub fn len(&self) -> usize {
        self.polygons.len()
    }

    /// Return true if this collection has no polygons.
    pub fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }

    /// Return an iterator over individual polygons in the sequence.
    pub fn iter(&self) -> PolygonsIter {
        PolygonsIter {
            points: &self.points,
            polygons: self.polygons.iter(),
        }
    }
}

impl Default for Polygons {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over polygons in a [Polygons] collection.
pub struct PolygonsIter<'a> {
    points: &'a [Point],
    polygons: Iter<'a, Range<usize>>,
}

impl<'a> Iterator for PolygonsIter<'a> {
    type Item = &'a [Point];

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(range) = self.polygons.next() {
            Some(&self.points[range.clone()])
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.polygons.size_hint()
    }
}

impl<'a> ExactSizeIterator for PolygonsIter<'a> {}

enum Direction {
    Clockwise,
    CounterClockwise,
}

/// Search the neighborhood of the pixel `center` in `mask` for a pixel with
/// a non-zero value, starting from `start` and in the order given by `dir`.
///
/// If `skip_first` is true, start the search from the next neighbor of `start`
/// in the order given by `dir`.
fn find_nonzero_neighbor(
    mask: &NdTensorView<i32, 2>,
    center: Point,
    start: Point,
    dir: Direction,
    skip_first: bool,
) -> Option<Point> {
    let neighbors = center.neighbors();
    let next_neighbor = |idx| match dir {
        Direction::Clockwise => (idx + 1) % neighbors.len(),
        Direction::CounterClockwise => {
            if idx == 0 {
                neighbors.len() - 1
            } else {
                idx - 1
            }
        }
    };

    let start_idx = neighbors
        .iter()
        .position(|&p| p == start)
        .map(|index| {
            if skip_first {
                next_neighbor(index)
            } else {
                index
            }
        })
        .unwrap();

    let mut idx = start_idx;
    loop {
        if mask[neighbors[idx].coord()] != 0 {
            return Some(neighbors[idx]);
        }
        idx = next_neighbor(idx);
        if idx == start_idx {
            break;
        }
    }

    None
}

/// Specifies which contours to extract from a mask in [find_contours].
pub enum RetrievalMode {
    /// Get only the outer-most contours.
    External,

    /// Retrieve all contours as a flat list without hierarchy information.
    List,
}

/// Find the contours of connected components in the binary image `mask`.
///
/// Returns a collection of the polygons of each component. The algorithm follows
/// the border of each component in counter-clockwise order.
///
/// This uses the algorithm from [1] (see Appendix 1), which is also the same
/// algorithm used in OpenCV's `findContours` function [2]. This function does
/// not currently implement the parts of the algorithm that discover the
/// hierarchical relationships between contours.
///
/// [1] Suzuki, Satoshi and Keiichi Abe. “Topological structural analysis of digitized binary
///     images by border following.” Comput. Vis. Graph. Image Process. 30 (1985): 32-46.
/// [2] https://docs.opencv.org/4.7.0/d3/dc0/group__imgproc__shape.html#gadf1ad6a0b82947fa1fe3c3d497f260e0
pub fn find_contours(mask: NdTensorView<i32, 2>, mode: RetrievalMode) -> Polygons {
    // Create a copy of the mask with zero-padding around the border. The
    // padding enables the algorithm to handle objects that touch the edge of
    // the mask.
    let padding = 1;
    let mut padded_mask = NdTensor::zeros([mask.rows() + 2 * padding, mask.cols() + 2 * padding]);
    for y in 0..mask.rows() {
        for x in 0..mask.cols() {
            // Clamp values in the copied mask to { 0, 1 } so the algorithm
            // below can use other values as part of its working.
            let value = mask[[y, x]].clamp(0, 1);
            padded_mask[[y + padding, x + padding]] = value;
        }
    }
    let mut mask = padded_mask;

    let mut contours = Polygons::new();

    // Points of current border.
    let mut border = Vec::new();

    // Sequential number of next border. Called `NBD` in the paper.
    let mut border_num = 1;

    // Value of last non-zero pixel visited on current row. See Algorithm 2 in
    // paper. This value is zero if we've not passed through any borders on the
    // current row yet, +ve if we're inside an outer border and -ve after
    // exiting the outer border.
    let mut last_nonzero_pixel;

    let outer_only = matches!(mode, RetrievalMode::External);

    for y in padding..mask.rows() - padding {
        let y = y as i32;
        last_nonzero_pixel = 0;

        for x in padding..mask.cols() - padding {
            let x = x as i32;

            let start_point = Point { y, x };
            let current = mask[start_point.coord()];
            if current == 0 {
                continue;
            }

            // Neighbor of current point to start searching for next pixel
            // along the border that begins at the current point.
            let mut start_neighbor = None;

            let prev_point = start_point.translate(0, -1);
            let next_point = start_point.translate(0, 1);

            // Test whether we are at the starting point of an unvisited border.
            if outer_only {
                if last_nonzero_pixel <= 0 && mask[prev_point.coord()] == 0 && current == 1 {
                    start_neighbor = Some(prev_point);
                }
            } else if mask[prev_point.coord()] == 0 && current == 1 {
                // This is a new outer border.
                start_neighbor = Some(prev_point);
            } else if current >= 1 && mask[next_point.coord()] == 0 {
                // This is a new hole border.
                start_neighbor = Some(next_point);
            }

            // Follow the border if we found a start point.
            if let Some(start_neighbor) = start_neighbor {
                border_num += 1;
                border.clear();

                let nonzero_start_neighbor = find_nonzero_neighbor(
                    &mask.view(),
                    start_point,
                    start_neighbor,
                    Direction::Clockwise,
                    false, // skip_first
                );

                if let Some(start_neighbor) = nonzero_start_neighbor {
                    let mut current_point = start_point;
                    let mut prev_neighbor = start_neighbor;

                    loop {
                        let next_point = find_nonzero_neighbor(
                            &mask.view(),
                            current_point,
                            prev_neighbor,
                            Direction::CounterClockwise,
                            true, // skip_first
                        );

                        // Determine if this is the right or left side of the
                        // border and set current pixel to -ve / +ve.
                        if mask[current_point.translate(0, 1).coord()] == 0 {
                            border.push(current_point);
                            mask[current_point.coord()] = -border_num;
                        } else if mask[current_point.coord()] == 1 {
                            border.push(current_point);
                            mask[current_point.coord()] = border_num;
                        }

                        if next_point == Some(start_point) && current_point == start_neighbor {
                            // We are back to the starting point of the border.
                            break;
                        }

                        // Go to the next pixel along the border.
                        prev_neighbor = current_point;
                        current_point = next_point.unwrap();
                    }
                } else {
                    // The current border consists of a single point. Mark it
                    // as being the right edge of a border.
                    border.push(start_point);
                    mask[start_point.coord()] = -border_num;
                }

                // Adjust coordinates to remove padding.
                for point in border.iter_mut() {
                    point.x -= padding as i32;
                    point.y -= padding as i32;
                }
                contours.push(&border);
            }

            last_nonzero_pixel = mask[start_point.coord()];
        }
    }

    contours
}

fn simplify_polyline_internal(
    points: &[Point],
    epsilon: f32,
    out_points: &mut Vec<Point>,
    keep_last: bool,
) {
    if points.len() <= 1 {
        if let Some(&point) = points.first() {
            out_points.push(point);
        }
        return;
    }

    // Find point furthest from the line segment through the first and last
    // points.
    let line_segment = Line::from_endpoints(*points.first().unwrap(), *points.last().unwrap());
    let inner_points = &points[1..points.len() - 1];
    let (max_index, max_dist) =
        inner_points
            .iter()
            .enumerate()
            .fold((0, 0.), |(max_i, max_dist), (i, &point)| {
                let dist = line_segment.distance(point);
                if dist >= max_dist {
                    (i + 1, dist)
                } else {
                    (max_i, max_dist)
                }
            });

    if max_dist > epsilon {
        // Recursively simplify polyline segments before and after pivot.
        simplify_polyline_internal(
            &points[..max_index + 1],
            epsilon,
            out_points,
            false, /* keep_last */
        );
        simplify_polyline_internal(&points[max_index..], epsilon, out_points, keep_last);
    } else {
        // Simplify current polyline to start and end points.
        out_points.push(line_segment.start);
        if keep_last {
            out_points.push(line_segment.end);
        }
    }
}

/// Return a simplified version of the polyline defined by `points`.
///
/// The result will be a subset of points from the input, which always includes
/// the first and last points.
///
/// `epsilon` specifies the maximum distance that any removed point may be from
/// the closest point on the simplified polygon.
///
/// This uses the Douglas-Peucker algorithm [1].
///
/// [1] https://en.wikipedia.org/wiki/Ramer–Douglas–Peucker_algorithm
pub fn simplify_polyline(points: &[Point], epsilon: f32) -> Vec<Point> {
    assert!(epsilon >= 0.);
    let mut result = Vec::new();
    simplify_polyline_internal(points, epsilon, &mut result, true /* keep_last */);
    result
}

/// Return a simplified version of the polygon defined by `points`.
///
/// This is very similar to [simplify_polyline] except that the input is
/// treated as a polygon where the last point implicitly connects to the first
/// point to close the shape.
pub fn simplify_polygon(points: &[Point], epsilon: f32) -> Vec<Point> {
    // Convert polygon to polyline.
    let mut polyline = points.to_vec();
    polyline.push(points[0]);

    // Simplify and convert polyline back to polygon.
    let mut simplified = simplify_polyline(&polyline, epsilon);
    simplified.truncate(simplified.len() - 1);

    simplified
}

/// Print out elements of a 2D grid for debugging.
#[allow(dead_code)]
fn print_grid<T: Display>(grid: NdTensorView<T, 2>) {
    for y in 0..grid.rows() {
        for x in 0..grid.cols() {
            print!("{:2} ", grid[[y, x]]);
        }
        println!();
    }
    println!();
}

/// Return the bounding box containing a set of points.
///
/// Panics if the point list is empty.
pub fn bounding_box(points: &[Point]) -> Rect {
    assert!(!points.is_empty(), "Point list must be non-empty");

    let mut left = points[0].x;
    let mut top = points[0].y;
    let mut right = left + 1;
    let mut bottom = top + 1;

    for point in points {
        left = left.min(point.x);
        right = right.max(point.x + 1);
        top = top.min(point.y);
        bottom = bottom.max(point.y + 1);
    }

    Rect::from_tlbr(top, left, bottom, right)
}

// Draw the outline of a rectangle `rect` with border width `width`.
//
// The outline is drawn such that the bounding box of the outermost pixels
// will be `rect`.
pub fn stroke_rect<T: Copy>(mut mask: NdTensorViewMut<T, 2>, rect: Rect, value: T, width: u32) {
    let width = width as i32;

    // Left edge
    fill_rect(
        mask.view_mut(),
        Rect::from_tlbr(rect.top(), rect.left(), rect.bottom(), rect.left() + width),
        value,
    );

    // Top edge (minus ends)
    fill_rect(
        mask.view_mut(),
        Rect::from_tlbr(
            rect.top(),
            rect.left() + width,
            rect.top() + width,
            rect.right() - width,
        ),
        value,
    );

    // Right edge
    fill_rect(
        mask.view_mut(),
        Rect::from_tlbr(
            rect.top(),
            rect.right() - width,
            rect.bottom(),
            rect.right(),
        ),
        value,
    );

    // Bottom edge (minus ends)
    fill_rect(
        mask.view_mut(),
        Rect::from_tlbr(
            rect.bottom() - width,
            rect.left() + width,
            rect.bottom(),
            rect.right() - width,
        ),
        value,
    );
}

/// Fill all points inside `rect` with the value `value`.
pub fn fill_rect<T: Copy>(mut mask: NdTensorViewMut<T, 2>, rect: Rect, value: T) {
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            mask[[y as usize, x as usize]] = value;
        }
    }
}

/// Return a copy of `p` with X and Y coordinates clamped to `[0, width)` and
/// `[0, height)` respectively.
fn clamp_to_bounds(p: Point, height: i32, width: i32) -> Point {
    Point::from_yx(
        p.y.clamp(0, height.saturating_sub(1).max(0)),
        p.x.clamp(0, width.saturating_sub(1).max(0)),
    )
}

/// Draw a non-antialiased line in an image.
pub fn draw_line<T: Copy>(mut image: NdTensorViewMut<T, 2>, line: Line, value: T) {
    // This function uses Breshan's line algorithm, with the implementation
    // in Pillow (https://pillow.readthedocs.io/en/stable/) used as a reference.

    let height: i32 = image.rows().try_into().unwrap();
    let width: i32 = image.cols().try_into().unwrap();

    let start = clamp_to_bounds(line.start, height, width);
    let end = clamp_to_bounds(line.end, height, width);

    let dx = (end.x - start.x).abs();
    let dy = (end.y - start.y).abs();

    let x_step = (end.x - start.x).signum();
    let y_step = (end.y - start.y).signum();

    let steps = dx.max(dy);
    let mut current = start;

    if x_step == 0 {
        // Vertical line
        let mut y = start.y;
        for _ in 0..steps {
            image[[y as usize, start.x as usize]] = value;
            y += y_step;
        }
    } else if y_step == 0 {
        // Horizontal line
        let mut x = start.x;
        for _ in 0..steps {
            image[[start.y as usize, x as usize]] = value;
            x += x_step;
        }
    } else if dx >= dy {
        // Horizontal slope
        let dy = dy * 2;
        let mut error = dy - dx;
        let dx = dx * 2;

        for _ in 0..steps {
            image[current.coord()] = value;
            if error >= 0 {
                current.y += y_step;
                error -= dx;
            }
            error += dy;
            current.x += x_step;
        }
    } else {
        // Vertical slope
        let dx = dx * 2;
        let mut error = dx - dy;
        let dy = dy * 2;

        for _ in 0..steps {
            image[current.coord()] = value;
            if error >= 0 {
                current.x += x_step;
                error -= dy;
            }
            error += dx;
            current.y += y_step;
        }
    }
}

/// Draw the outline of a non anti-aliased polygon in an image.
pub fn draw_polygon<T: Copy>(mut image: NdTensorViewMut<T, 2>, poly: &[Point], value: T) {
    if poly.is_empty() {
        return;
    }

    for (&start, &end) in zip(poly.iter(), poly.iter().skip(1)) {
        draw_line(image.view_mut(), Line::from_endpoints(start, end), value);
    }
    draw_line(
        image,
        Line::from_endpoints(*poly.last().unwrap(), *poly.first().unwrap()),
        value,
    );
}

/// Return the sorted subset of points from `poly` that form a convex hull
/// containing `poly`.
pub fn convex_hull(poly: &[Point]) -> Vec<Point> {
    // See https://en.wikipedia.org/wiki/Graham_scan

    let mut hull = Vec::new();

    // Find lowest and left-most point.
    let min_point = match poly.iter().min_by_key(|p| (-p.y, p.x)) {
        Some(p) => p,
        None => {
            return hull;
        }
    };

    // FIXME - Should `min_point` be removed from the list? It leads to NaN
    // outputs from `angle` when angle is called with `p == min_point`.
    //
    // TODO - Break ties if multiple points form the same angle, by preferring
    // the furthest point.

    // Compute cosine of angle between the vector `p - min_point` and the X axis.
    let angle = |p: Point| {
        let dy = p.y - min_point.y;
        let dx = p.x - min_point.x;
        let x_axis = Vec2::from_yx(0., 1.);
        Vec2::from_yx(dy as f32, dx as f32).normalized().dot(x_axis)
    };

    // Sort points by angle between `point - min_point` and X axis.
    let mut sorted_points = poly.to_vec();
    sorted_points.sort_by(|&a, &b| f32::total_cmp(&angle(a), &angle(b)));

    // Visit sorted points and keep the sequence that can be followed without
    // making any clockwise turns.
    for &p in sorted_points.iter() {
        while hull.len() >= 2 {
            let [prev2, prev] = [hull[hull.len() - 2], hull[hull.len() - 1]];
            let ac = Vec2::from_points(prev2, p);
            let bc = Vec2::from_points(prev, p);
            let turn_dir = ac.cross_product_norm(bc);
            if turn_dir > 0. {
                // Last three points form a counter-clockwise turn.
                break;
            }
            hull.pop();
        }
        hull.push(p);
    }

    hull
}

/// An oriented rectangle.
#[derive(Copy, Clone, Debug)]
pub struct RotatedRect {
    // Centroid of the rect.
    center: Vec2,

    // Unit-length vector indicating the "up" direction for this rect.
    up_axis: Vec2,

    // Extent of the rect along the axis perpendicular to `up`.
    width: f32,

    // Extent of the rect along the `up` axis.
    height: f32,
}

impl RotatedRect {
    /// Construct a new RotatedRect with a given `center`, up direction and
    /// dimensions.
    ///
    /// `up_axis` must be a normalized (unit-length) vector.
    pub fn new(center: Vec2, up_axis: Vec2, width: f32, height: f32) -> RotatedRect {
        RotatedRect {
            center,
            up_axis,
            width,
            height,
        }
    }

    /// Return the coordinates of the rect's corners.
    ///
    /// The corners are returned in clockwise order starting from the corner
    /// that is the top-left when the rect has no rotation (ie. the "up" axis
    /// has XY coordinates [0, 1]).
    pub fn corners(&self) -> [Point; 4] {
        let par_offset = self.up_axis.perpendicular() * (self.width / 2.);
        let perp_offset = self.up_axis * (self.height / 2.);

        let coords: [Vec2; 4] = [
            self.center - perp_offset - par_offset,
            self.center - perp_offset + par_offset,
            self.center + perp_offset + par_offset,
            self.center + perp_offset - par_offset,
        ];

        coords.map(|v| Point::from_yx(v.y as i32, v.x as i32))
    }

    /// Return the axis-aligned bounding rect which contains this rotated rect.
    pub fn bounding_rect(&self) -> Rect {
        let corners = self.corners();

        let mut xs = corners.map(|p| p.x);
        xs.sort();

        let mut ys = corners.map(|p| p.y);
        ys.sort();

        Rect::from_tlbr(ys[0], xs[0], ys[3], xs[3])
    }

    /// Return the centroid of the rect.
    pub fn center(&self) -> Vec2 {
        self.center
    }

    /// Return the normalized vector that indicates the "up" direction for
    /// this rect.
    pub fn up_axis(&self) -> Vec2 {
        self.up_axis
    }

    /// Return the extent of the rect along the axis perpendicular to `self.up_axis()`.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Return the extent of the rect along `self.up_axis()`.
    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn area(&self) -> f32 {
        self.height * self.width
    }

    /// Set the extents of this rect. `width` and `height` must be >= 0.
    pub fn resize(&mut self, width: f32, height: f32) {
        assert!(width >= 0. && height >= 0.);
        self.width = width;
        self.height = height;
    }
}

/// Return the rotated rectangle with minimum area which contains `points`.
///
/// Returns `None` if `points` contains fewer than 2 points.
pub fn min_area_rect(points: &[Point]) -> Option<RotatedRect> {
    // See "Exhaustive Search Algorithm" in
    // https://www.geometrictools.com/Documentation/MinimumAreaRectangle.pdf.

    let hull = convex_hull(points);

    // Iterate over each edge of the polygon and find the smallest bounding
    // rect where one of the rect's edges aligns with the polygon edge. Keep
    // the rect that has the smallest area over all edges.
    let mut min_rect: Option<RotatedRect> = None;
    for (&edge_start, &edge_end) in zip(hull.iter(), hull.iter().cycle().skip(1)) {
        // Project polygon points onto axes that are parallel and perpendicular
        // to the current edge. The maximum distance between the projected
        // points gives the width and height of the bounding rect.

        let par_axis = Vec2::from_points(edge_start, edge_end).normalized();

        // nb. Perpendicular axis points into the hull.
        let perp_axis = -par_axis.perpendicular();

        let (min_par, max_par, max_perp): (f32, f32, f32) = hull.iter().fold(
            (f32::MAX, f32::MIN, f32::MIN),
            |(min_par, max_par, max_perp), point| {
                let d = Vec2::from_points(edge_start, *point);
                let par_proj = par_axis.dot(d);
                let perp_proj = perp_axis.dot(d);
                (
                    min_par.min(par_proj),
                    max_par.max(par_proj),
                    max_perp.max(perp_proj),
                )
            },
        );

        let height = max_perp;
        let width = max_par - min_par;
        let area = height * width;

        if area < min_rect.map(|r| r.area()).unwrap_or(f32::MAX) {
            let center = Vec2::from_yx(edge_start.y as f32, edge_start.x as f32)
                + (par_axis * ((min_par + max_par) / 2.))
                + (perp_axis * (height / 2.));
            min_rect = Some(RotatedRect::new(
                center, /* up_axis */ perp_axis, width, height,
            ))
        }
    }

    min_rect
}

#[cfg(test)]
mod tests {
    use std::iter::zip;

    use super::{
        bounding_box, convex_hull, draw_polygon, fill_rect, find_contours, min_area_rect,
        print_grid, simplify_polygon, simplify_polyline, stroke_rect, Line, Point, Rect,
        RetrievalMode, RotatedRect, Vec2,
    };
    use crate::tensor::{MatrixLayout, NdTensor, NdTensorLayout, NdTensorView, NdTensorViewMut};
    use crate::test_util::ApproxEq;

    /// Return a list of the points on the border of `rect`, in counter-clockwise
    /// order starting from the top-left corner.
    ///
    /// If `omit_corners` is true, the corner points of the rect are not
    /// included.
    fn border_points(rect: Rect, omit_corners: bool) -> Vec<Point> {
        let mut points = Vec::new();

        let left_range = if omit_corners {
            rect.top() + 1..rect.bottom() - 1
        } else {
            rect.top()..rect.bottom()
        };

        // Left edge
        for y in left_range.clone() {
            points.push(Point::from_yx(y, rect.left()));
        }

        // Bottom edge
        for x in rect.left() + 1..rect.right() - 1 {
            points.push(Point::from_yx(rect.bottom() - 1, x));
        }

        // Right edge
        for y in left_range.rev() {
            points.push(Point::from_yx(y, rect.right() - 1));
        }

        // Top edge
        for x in (rect.left() + 1..rect.right() - 1).rev() {
            points.push(Point::from_yx(rect.top(), x));
        }

        points
    }

    /// Set the elements of a grid listed in `points` to `value`.
    #[allow(dead_code)]
    fn plot_points<T: Copy>(mut grid: NdTensorViewMut<T, 2>, points: &[Point], value: T) {
        for point in points {
            grid[point.coord()] = value;
        }
    }

    /// Plot the 1-based indices of points in `points` on a grid. `step` is the
    /// increment value for each plotted point.
    #[allow(dead_code)]
    fn plot_point_indices<T: std::ops::AddAssign + Copy + Default>(
        mut grid: NdTensorViewMut<T, 2>,
        points: &[Point],
        step: T,
    ) {
        let mut value = T::default();
        value += step;
        for point in points {
            grid[point.coord()] = value;
            value += step;
        }
    }

    /// Return coordinates of all points in `grid` with a non-zero value.
    fn nonzero_points<T: Default + PartialEq>(grid: NdTensorView<T, 2>) -> Vec<Point> {
        let mut points = Vec::new();
        for y in 0..grid.rows() {
            for x in 0..grid.cols() {
                if grid[[y, x]] != T::default() {
                    points.push(Point::from_yx(y as i32, x as i32))
                }
            }
        }
        points
    }

    /// Create a 2D NdTensor from an MxN nested array.
    fn image_from_2d_array<const M: usize, const N: usize>(xs: [[i32; N]; M]) -> NdTensor<i32, 2> {
        let mut image = NdTensor::zeros([M, N]);
        for y in 0..M {
            for x in 0..N {
                image[[y, x]] = xs[y][x];
            }
        }
        image
    }

    /// Compare two single-channel images with i32 pixel values.
    fn compare_images(a: NdTensorView<i32, 2>, b: NdTensorView<i32, 2>) {
        assert_eq!(a.rows(), b.rows());
        assert_eq!(a.cols(), b.cols());

        for y in 0..a.rows() {
            for x in 0..a.cols() {
                if a[[y, x]] != b[[y, x]] {
                    print_grid(a);
                    panic!("mismatch at coord [{}, {}]", y, x);
                }
            }
        }
    }

    /// Convert a slice of `[y, x]` coordinates to `Point`s
    fn points_from_coords(coords: &[[i32; 2]]) -> Vec<Point> {
        coords.iter().map(|[y, x]| Point::from_yx(*y, *x)).collect()
    }

    /// Convery an array of `[y, x]` coordinates to `Point`s
    fn points_from_n_coords<const N: usize>(coords: [[i32; 2]; N]) -> [Point; N] {
        coords.map(|[y, x]| Point::from_yx(y, x))
    }

    #[test]
    fn test_bounding_box() {
        let rect = Rect::from_tlbr(5, 5, 10, 10);
        let border = border_points(rect, false /* omit_corners */);
        assert_eq!(bounding_box(&border), rect);
    }

    #[test]
    fn test_convex_hull() {
        struct Case {
            points: &'static [[i32; 2]],
            hull: &'static [[i32; 2]],
        }

        let cases = [
            // Simple square. The hull is a re-ordering of the input.
            Case {
                points: &[[0, 0], [0, 4], [4, 4], [4, 0]],
                hull: &[[4, 0], [0, 0], [0, 4], [4, 4]],
            },
            // Square with an indent on each edge. The hull is just a rect.
            Case {
                points: &[
                    // Top
                    [0, 0],
                    [1, 2],
                    [0, 4],
                    // Right
                    [2, 3],
                    [4, 4],
                    // Bottom
                    [3, 2],
                    [4, 0],
                    // Left
                    [2, 1],
                ],

                // Hull starts with lowest, left-most corner then proceeds
                // clockwise.
                hull: &[[4, 0], [0, 0], [0, 4], [4, 4]],
            },
        ];

        for case in cases {
            let points = points_from_coords(case.points);
            let expected_hull = points_from_coords(case.hull);

            let hull = convex_hull(&points);

            assert_eq!(hull, expected_hull);
        }
    }

    #[test]
    fn test_draw_polygon() {
        struct Case {
            points: &'static [[i32; 2]],
            expected: NdTensor<i32, 2>,
        }

        let cases = [
            // A simple rect: Straight lines in each direction
            Case {
                points: &[[0, 0], [0, 4], [4, 4], [4, 0]],
                expected: image_from_2d_array([
                    [1, 1, 1, 1, 1],
                    [1, 0, 0, 0, 1],
                    [1, 0, 0, 0, 1],
                    [1, 0, 0, 0, 1],
                    [1, 1, 1, 1, 1],
                ]),
            },
            // Slopes in each direction.
            Case {
                points: &[[0, 2], [2, 0], [4, 2], [2, 4]],
                expected: image_from_2d_array([
                    [0, 0, 1, 0, 0],
                    [0, 1, 0, 1, 0],
                    [1, 0, 0, 0, 1],
                    [0, 1, 0, 1, 0],
                    [0, 0, 1, 0, 0],
                ]),
            },
            // Steep slopes in each direction.
            Case {
                points: &[[0, 2], [2, 1], [4, 2], [2, 3]],
                expected: image_from_2d_array([
                    [0, 0, 1, 0, 0],
                    [0, 1, 1, 0, 0],
                    [0, 1, 0, 1, 0],
                    [0, 0, 1, 1, 0],
                    [0, 0, 1, 0, 0],
                ]),
            },
        ];

        for case in cases {
            let points: Vec<_> = case
                .points
                .iter()
                .map(|[y, x]| Point::from_yx(*y, *x))
                .collect();

            let mut image = NdTensor::zeros(case.expected.shape());
            draw_polygon(image.view_mut(), &points, 1);
            compare_images(image.view(), case.expected.view());
        }
    }

    #[test]
    fn test_find_contours_in_empty_mask() {
        struct Case {
            size: [usize; 2],
        }

        let cases = [
            Case { size: [0, 0] },
            Case { size: [1, 1] },
            Case { size: [10, 10] },
        ];

        for case in cases {
            let mask = NdTensor::zeros(case.size);
            let contours = find_contours(mask.view(), RetrievalMode::List);
            assert_eq!(contours.len(), 0);
        }
    }

    #[test]
    fn test_find_contours_single_rect() {
        struct Case {
            rect: Rect,
            value: i32,
        }

        let cases = [
            Case {
                rect: Rect::from_tlbr(5, 5, 10, 10),
                value: 1,
            },
            // Values > 1 in the mask are clamped to 1, so they don't affect
            // the contours found.
            Case {
                rect: Rect::from_tlbr(5, 5, 10, 10),
                value: 2,
            },
            // Values < 0 are clamped to 0 and ignored.
            Case {
                rect: Rect::from_tlbr(5, 5, 10, 10),
                value: -2,
            },
        ];

        for case in cases {
            let mut mask = NdTensor::zeros([20, 20]);
            fill_rect(mask.view_mut(), case.rect, case.value);

            let contours = find_contours(mask.view(), RetrievalMode::List);

            if case.value > 0 {
                assert_eq!(contours.len(), 1);
                let border = contours.iter().next().unwrap();
                assert_eq!(border, border_points(case.rect, false /* omit_corners */));
            } else {
                assert!(contours.is_empty());
            }
        }
    }

    #[test]
    fn test_find_contours_rect_touching_frame() {
        let mut mask = NdTensor::zeros([5, 5]);
        let rect = Rect::from_tlbr(0, 0, 5, 5);
        fill_rect(mask.view_mut(), rect, 1);

        let contours = find_contours(mask.view(), RetrievalMode::List);
        assert_eq!(contours.len(), 1);

        let border = contours.iter().next().unwrap();
        assert_eq!(border, border_points(rect, false /* omit_corners */));
    }

    #[test]
    fn test_find_contours_hollow_rect() {
        let mut mask = NdTensor::zeros([20, 20]);
        let rect = Rect::from_tlbr(5, 5, 12, 12);
        stroke_rect(mask.view_mut(), rect, 1, 2);

        let contours = find_contours(mask.view(), RetrievalMode::List);

        // There should be two contours: one for the outer border of the rect
        // and one for the inner "hole" border.
        assert_eq!(contours.len(), 2);

        // Check outer border.
        let mut contours_iter = contours.iter();
        let outer_border = contours_iter.next().unwrap();
        let inner_border = contours_iter.next().unwrap();
        assert_eq!(outer_border, border_points(rect, false /* omit_corners */));

        // Check hole border.
        let inner_rect = rect.adjust_tlbr(1, 1, -1, -1);
        let mut expected_inner_border = border_points(inner_rect, true /* omit_corners */);

        // Due to the way the algorithm works, hole border points are returned
        // in the opposite order (clockwise instead of counter-clockwise) to
        // outer border points, and the start position is shifted by one.
        expected_inner_border.reverse(); // CCW => CW
        expected_inner_border.rotate_right(1);

        assert_eq!(inner_border, expected_inner_border);
    }

    #[test]
    fn test_find_contours_external() {
        let mut mask = NdTensor::zeros([20, 20]);
        let rect = Rect::from_tlbr(5, 5, 12, 12);
        stroke_rect(mask.view_mut(), rect, 1, 2);

        let contours = find_contours(mask.view(), RetrievalMode::External);

        // There should only be one, outermost contour.
        assert_eq!(contours.len(), 1);
        let outer_border = contours.iter().next().unwrap();
        assert_eq!(outer_border, border_points(rect, false /* omit_corners */));
    }

    #[test]
    fn test_find_contours_single_point() {
        let mut mask = NdTensor::zeros([20, 20]);
        mask[[5, 5]] = 1;

        let contours = find_contours(mask.view(), RetrievalMode::List);
        assert_eq!(contours.len(), 1);

        let border = contours.iter().next().unwrap();
        assert_eq!(border, [Point::from_yx(5, 5)]);
    }

    #[test]
    fn test_find_contours_many_rects() {
        let mut mask = NdTensor::zeros([20, 20]);

        let rects = [
            Rect::from_tlbr(5, 5, 10, 10),
            Rect::from_tlbr(15, 15, 18, 18),
        ];
        for rect in rects {
            fill_rect(mask.view_mut(), rect, 1);
        }

        let contours = find_contours(mask.view(), RetrievalMode::List);
        assert_eq!(contours.len(), rects.len());

        for (border, rect) in zip(contours.iter(), rects.iter()) {
            assert_eq!(border, border_points(*rect, false /* omit_corners */));
        }
    }

    #[test]
    fn test_find_contours_nested_rects() {
        let mut mask = NdTensor::zeros([15, 15]);

        let rects = [Rect::from_tlbr(5, 5, 11, 11), Rect::from_tlbr(7, 7, 9, 9)];
        for rect in rects {
            stroke_rect(mask.view_mut(), rect, 1, 1);
        }

        let contours = find_contours(mask.view(), RetrievalMode::List);
        assert_eq!(contours.len(), rects.len());

        for (border, rect) in zip(contours.iter(), rects.iter()) {
            assert_eq!(border, border_points(*rect, false /* omit_corners */));
        }
    }

    #[test]
    fn test_line_distance() {
        struct Case {
            start: Point,
            end: Point,
            point: Point,
            dist: f32,
        }

        // TODO - Test cases where intercept is beyond start/end of line.
        let cases = [
            // Single point
            Case {
                start: Point::default(),
                end: Point::default(),
                point: Point::from_yx(3, 4),
                dist: 5.,
            },
            // Horizontal line
            Case {
                start: Point::from_yx(5, 2),
                end: Point::from_yx(5, 10),
                point: Point::from_yx(8, 5),
                dist: 3.,
            },
            // Vertical line
            Case {
                start: Point::from_yx(5, 3),
                end: Point::from_yx(10, 3),
                point: Point::from_yx(8, 5),
                dist: 2.,
            },
            // Line with +ve gradient
            Case {
                start: Point::default(),
                end: Point::from_yx(5, 5),
                point: Point::from_yx(4, 0),
                dist: (8f32).sqrt(), // Closest point is at (2, 2)
            },
            // Line with -ve gradient
            Case {
                start: Point::default(),
                end: Point::from_yx(5, -5),
                point: Point::from_yx(4, 0),
                dist: (8f32).sqrt(), // Closest point is at (2, -2)
            },
            // Point below line
            Case {
                start: Point::default(),
                end: Point::from_yx(5, 5),
                point: Point::from_yx(0, 4),
                dist: (8f32).sqrt(), // Closest point is at (2, 2)
            },
            // Point beyond end of horizontal line
            Case {
                start: Point::from_yx(5, 2),
                end: Point::from_yx(5, 5),
                point: Point::from_yx(5, 10),
                dist: 5.,
            },
        ];

        for case in cases {
            let line = Line::from_endpoints(case.start, case.end);
            let dist = line.distance(case.point);
            assert!(
                dist.approx_eq(case.dist),
                "line {:?}, {:?} point {:?} actual {} expected {}",
                line.start,
                line.end,
                case.point,
                dist,
                case.dist
            );
        }
    }

    #[test]
    fn test_line_overlap() {
        struct Case {
            a: (i32, i32),
            b: (i32, i32),
            overlap: i32,
        }

        let cases = [
            // No overlap
            Case {
                a: (0, 10),
                b: (15, 20),
                overlap: 0,
            },
            // End of `a` overlaps start of `b`
            Case {
                a: (0, 10),
                b: (5, 15),
                overlap: 5,
            },
            // `a` overlaps all of `b`
            Case {
                a: (0, 10),
                b: (2, 8),
                overlap: 6,
            },
            // `a` and `b` start together, but `a` is shorter
            Case {
                a: (0, 5),
                b: (0, 10),
                overlap: 5,
            },
        ];

        for case in cases {
            // Horizontal lines
            let a = Line::from_endpoints(Point::from_yx(0, case.a.0), Point::from_yx(0, case.a.1));
            let b = Line::from_endpoints(Point::from_yx(0, case.b.0), Point::from_yx(0, case.b.1));
            assert_eq!(a.horizontal_overlap(b), case.overlap);
            assert_eq!(b.horizontal_overlap(a), case.overlap);

            // Vertical lines
            let a = Line::from_endpoints(Point::from_yx(case.a.0, 0), Point::from_yx(case.a.1, 0));
            let b = Line::from_endpoints(Point::from_yx(case.b.0, 0), Point::from_yx(case.b.1, 0));
            assert_eq!(a.vertical_overlap(b), case.overlap);
            assert_eq!(b.vertical_overlap(a), case.overlap);
        }
    }

    #[test]
    fn test_min_area_rect() {
        struct Case {
            points: Vec<Point>,
            expected: [Point; 4],
        }

        let cases = [
            // Axis-aligned rect
            Case {
                points: points_from_coords(&[[0, 0], [0, 4], [4, 4], [4, 0]]),
                expected: points_from_n_coords([[4, 0], [0, 0], [0, 4], [4, 4]]),
            },
        ];

        for case in cases {
            let min_rect = min_area_rect(&case.points).unwrap();
            assert_eq!(min_rect.corners(), case.expected);
        }
    }

    #[test]
    fn test_rect_clamp() {
        struct Case {
            rect: Rect,
            boundary: Rect,
            expected: Rect,
        }

        let cases = [
            Case {
                rect: Rect::from_tlbr(-5, -10, 100, 200),
                boundary: Rect::from_tlbr(0, 0, 50, 100),
                expected: Rect::from_tlbr(0, 0, 50, 100),
            },
            Case {
                rect: Rect::from_tlbr(5, 10, 40, 80),
                boundary: Rect::from_tlbr(0, 0, 50, 100),
                expected: Rect::from_tlbr(5, 10, 40, 80),
            },
        ];

        for case in cases {
            assert_eq!(case.rect.clamp(case.boundary), case.expected);
        }
    }

    #[test]
    fn test_rotated_rect_corners() {
        let r = RotatedRect::new(Vec2::from_yx(5., 5.), Vec2::from_yx(1., 0.), 5., 5.);
        let expected = points_from_n_coords([[2, 2], [2, 7], [7, 7], [7, 2]]);
        assert_eq!(r.corners(), expected);
    }

    #[test]
    fn test_rotated_rect_resize() {
        let mut r = RotatedRect::new(Vec2::from_yx(5., 5.), Vec2::from_yx(1., 0.), 5., 5.);
        assert_eq!(r.area(), 25.);

        r.resize(3., 7.);

        assert_eq!(r.width(), 3.);
        assert_eq!(r.height(), 7.);
        assert_eq!(r.area(), 21.);
    }

    #[test]
    fn test_stroke_rect() {
        let mut mask = NdTensor::zeros([10, 10]);
        let rect = Rect::from_tlbr(4, 4, 9, 9);

        stroke_rect(mask.view_mut(), rect, 1, 1);
        let points = nonzero_points(mask.view());

        assert_eq!(bounding_box(&points), rect);
    }

    // TODO - Additional test cases for simplifying polygon:
    //
    // - Circle with varying epsilon values
    #[test]
    fn test_simplify_polyline() {
        struct Case {
            poly: Vec<Point>,
            epsilon: f32,
            simplified: Vec<Point>,
        }

        let cases = [
            // Single point
            Case {
                poly: vec![Point::from_yx(0, 0)],
                epsilon: 0.1,
                simplified: vec![Point::from_yx(0, 0)],
            },
            // Line of 2 points
            Case {
                poly: vec![Point::from_yx(5, 2), Point::from_yx(3, 5)],
                epsilon: 0.1,
                simplified: vec![Point::from_yx(5, 2), Point::from_yx(3, 5)],
            },
            // Line of 3 points
            Case {
                poly: vec![
                    Point::from_yx(5, 2),
                    Point::from_yx(5, 3),
                    Point::from_yx(5, 4),
                ],
                epsilon: 0.1,
                simplified: vec![Point::from_yx(5, 2), Point::from_yx(5, 4)],
            },
            // Line of 4 points
            Case {
                poly: vec![
                    Point::from_yx(5, 2),
                    Point::from_yx(5, 3),
                    Point::from_yx(5, 4),
                    Point::from_yx(5, 5),
                ],
                epsilon: 0.1,
                simplified: vec![Point::from_yx(5, 2), Point::from_yx(5, 5)],
            },
            // Boundary points of rect
            Case {
                poly: border_points(Rect::from_tlbr(4, 4, 9, 9), false /* omit_corners */),
                epsilon: 0.1,
                simplified: [[4, 4], [8, 4], [8, 8], [4, 8], [4, 5]]
                    .map(|[y, x]| Point::from_yx(y, x))
                    .into_iter()
                    .collect(),
            },
        ];

        for case in cases {
            let simplified = simplify_polyline(&case.poly, case.epsilon);
            assert_eq!(&simplified, &case.simplified);
        }
    }

    #[test]
    fn test_simplify_polygon() {
        struct Case {
            poly: Vec<Point>,
            epsilon: f32,
            simplified: Vec<Point>,
        }

        // Since `simplify_polygon` is a thin wrapper around `simplify_polyline`,
        // so we only have a few cases to cover the differences here.
        let cases = [Case {
            poly: border_points(Rect::from_tlbr(4, 4, 9, 9), false /* omit_corners */),
            epsilon: 0.1,
            simplified: [[4, 4], [8, 4], [8, 8], [4, 8]]
                .map(|[y, x]| Point::from_yx(y, x))
                .into_iter()
                .collect(),
        }];

        for case in cases {
            let simplified = simplify_polygon(&case.poly, case.epsilon);
            assert_eq!(&simplified, &case.simplified);
        }
    }
}
