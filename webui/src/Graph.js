import _ from "lodash";
import React from "react";
import { fmt_date_unix, fmt_size, def } from "./utils.js";
import Dygraph from "dygraphs";
import classNames from "classnames";

export default class Graph extends React.Component {
    target = null
    state = {}

    constructor() {
        super()
        this.onContextMenu = this.onContextMenu.bind( this );
    }

    componentDidMount() {
        this.forceRefresh();
    }

    componentDidUpdate( prev_props, prev_state ) {
        this.graph.updateOptions( this.getOptions() );
        this.refresh( prev_props );
    }

    getData() {
        if( this.cache && this.cachedRawData === this.props.data ) {
            return this.cache;
        }

        let output = {
            xs: [],
            ys: [],
            x_label: def( this.props.x_label, this.props.x_accessor || "x" ),
            y_labels: []
        };

        if( _.isArray( this.props.data ) ) {
            throw "Unimplemented!";
        } else {
            output.xs = this.props.data[ this.props.x_accessor || "xs" ];

            if( output.xs === undefined ) {
                throw "Missing `x_accessor`!"
            }

            if( this.props.y_accessors !== undefined ) {
                _.each( this.props.y_accessors, (y_accessor, index) => {
                    output.ys.push( this.props.data[ y_accessor ] );
                    output.y_labels.push( (this.props.y_labels || {})[ index ] || y_accessor );
                });
            } else {
                const y_accessor = this.props.y_accessor || "ys";
                if( this.props.data[ y_accessor ] === undefined ) {
                    throw "Missing `" + y_accessor + "` Y array!";
                }

                output.ys.push( this.props.data[ y_accessor ] );
                output.y_labels.push( def( this.props.y_label, this.props.y_accessor || "Y" ) );
            }
        }

        this.cache = output;
        this.cachedRawData = this.props.data;
        return output;
    }

    getCsvData() {
        const data = this.getData();

        let output = "";
        output += data.x_label + ",";
        _.each( data.y_labels, y_label => {
            output += y_label + ",";
        });
        output = output.slice( 0, output.length - 1 );
        output += "\n";

        for( let n_row = 0; n_row < data.xs.length; ++n_row ) {
            const x = data.xs[ n_row ];
            output += x + ",";

            let offset = 0;
            for( let n_column = 0; n_column < data.ys.length; ++n_column ) {
                const raw_value = data.ys[ n_column ][ n_row ] || 0;
                const y = raw_value + offset;
                output += y + ",";
                if( this.props.fill ) {
                    offset += y;
                }
            }

            output = output.slice( 0, output.length - 1 );
            output += "\n";
        }

        return output;
    }

    getZoom( props ) {
        props = props || this.props;

        const xs = this.getData().xs;
        let x0 = xs[0];
        let x1 = xs[xs.length - 1];
        if( _.isNumber( props.x0 ) ) {
            x0 = props.x0;
        } else if( _.isNumber( this.state.x0 ) ) {
            x0 = this.state.x0;
        } else {
        }

        if( _.isNumber( props.x1 ) ) {
            x1 = props.x1;
        } else if( _.isNumber( this.state.x1 ) ) {
            x1 = this.state.x1;
        }

        return [x0, x1];
    }

    getOptions() {
        const height = def( this.props.height, 300 );
        let options = {
            includeZero: def( this.props.includeZero, false ),
            axes: {
                x: {
                    axisLabelFormatter: this.formatX.bind( this )
                },
                y: {
                    axisLabelFormatter: (value, gran, opts, dygraph) => {
                        return fmt_size( value, false );
                    },
                    drawAxis: def( this.props.drawY, true )
                }
            },
            ylabel: this.props.y_label,
            height,
            fillGraph: this.props.fill,
            legendFormatter: this.legendFormatter.bind( this ),
            zoomCallback: (min, max, y_ranges) => {
                const old = this.getZoom();
                if( min === old[0] && max === old[1] ) {
                    return;
                }

                this.setState( {x0: min, x1: max} );
                if( this.props.onZoom ) {
                    this.props.onZoom( min, max );
                }
            },
            fillAlpha: def( this.props.fillAlpha, 0.15 ),
            hideOverlayOnMouseOut: false,
            highlightCircleSize: 3,
            drawHighlightPointCallback: (g, series_name, ctx, cx, cy, color, point_size) => {
                ctx.beginPath();
                ctx.fillStyle = "#de7b18ff";
                ctx.arc( cx, cy, point_size, 0, 2 * Math.PI, false );
                ctx.fill();
                ctx.strokeStyle = "#de7b18ff";
                ctx.lineWidth = 1;
                ctx.moveTo( cx, cy );
                ctx.lineTo( cx, cy + height );
                ctx.moveTo( cx, cy );
                ctx.lineTo( cx, cy - height );
                ctx.stroke();
                ctx.closePath();
            },
            dateWindow: this.getZoom()
        };

        if( this.target ) {
            options.width = this.getWidth();
        }

        return options;
    }

    getWidth() {
        if( this.target.parentElement.offsetWidth > 0 ) {
            return this.target.parentElement.offsetWidth;
        } else {
            return null;
        }
    }

    refresh( prev_props ) {
        const optionsChanged =
            prev_props &&
            (
                !_.isEqual( _.omit( prev_props, "data", "x0", "x1" ), _.omit( this.props, "data", "x0", "x1" ) ) ||
                this.getZoom() != this.getZoom( prev_props )
            );

        const shouldResize = this.width != this.getWidth();
        const shouldForceRefresh =
            !(prev_props.data === this.props.data ||
            _.isEqual( prev_props.data, this.props.data )) ||
            !_.isEqual( prev_props.y_accessors, this.props.y_accessors ) ||
            prev_props.fill !== this.props.fill;

        if( shouldForceRefresh || shouldResize ) {
            this.forceRefresh();
            return;
        }

        if( optionsChanged ) {
            this.graph.updateOptions( this.getOptions() );
        }
    }

    forceRefresh() {
        if( !this.target ) {
            return;
        }

        const data = this.getCsvData();
        const options = this.getOptions();
        this.graph = new Dygraph( this.target, data, options );
        this.width = options.width;
    }

    render() {
        return (
            <div style={{display: "flex"}}>
                <div className={classNames("dygraph", this.props.className)} ref={ ref => {
                    if( ref == null ) {
                        return;
                    }

                    this.target = ref;
                    this.target.removeEventListener( "contextmenu", this.onContextMenu );
                    this.target.addEventListener( "contextmenu", this.onContextMenu );
                }} />
            </div>
        );
    }

    onContextMenu( event ) {
        event.preventDefault();

        var c = this.graph.eventToDomCoords( event );
        var x = this.graph.toDataXCoord( c[0] );
        if( this.props.onRightClick ) {
            const zoom = this.getZoom();
            this.props.onRightClick({
                event,
                x,
                x0: zoom[0],
                x1: zoom[1]
            });
        }
        return false;
    }

    formatX( value ) {
        if( this.props.xUnit === "unix_timestamp" ) {
            return fmt_date_unix( value );
        } else if( this.props.xUnit instanceof Function ) {
            return this.props.xUnit( value );
        } else {
            return value;
        }
    }

    legendFormatter( cell ) {
        const data = this.getData();
        const labels = data.y_labels;
        let sum = 0;
        let index = 0;
        let prev = 0;
        const entries = _.map( cell.series, series => {
            let entry = "";
            entry += " ";
            entry += "<span style=\"color: " + series.color + "\">";
            entry += labels[ index ];
            const value = this.props.fill ? series.y - prev : series.y;
            if( cell.series.length > 1 ) {
                entry += " (" + fmt_size( value ) + ")";
            }
            entry += "</span>";
            index += 1;
            sum += value;
            prev = series.y;
            return [series.y, entry];
        });
        entries.sort( (a, b) => {
            return b[0] - a[0];
        });

        let output = "";
        if( this.props.x_legend_formatter ) {
            output += this.props.x_legend_formatter( cell.x );
        } else {
            output += this.formatX( cell.x );
        }

        if( def( this.props.showYValue, true ) ) {
            output += " (";
            output += fmt_size( sum );
            output += ")";
        }
        output += "<br />";
        output += _.map( entries, entry => entry[1] ).join( " " );

        return output;
    }
}
